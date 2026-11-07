#!/usr/bin/env python3
"""Fail-closed immutable release gates. Network operations are explicit subcommands."""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tomllib
from urllib.parse import urlencode

RUST_VERSION = "1.99.0"
REQUIRED_JOBS = ("Quality gates", "Dependency policy", "Spelling", "Release gates")
TARGETS = ("x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu")


class ReleaseError(ValueError):
    """A release precondition was not satisfied."""


def require(condition, message):
    if not condition:
        raise ReleaseError(message)


def command(*args, cwd=None):
    return subprocess.check_output(args, cwd=cwd, text=True).strip()


def gh_api(path):
    return json.loads(command("gh", "api", "-H", "Accept: application/vnd.github+json", path))


def pages(path, key):
    result = []
    separator = "&" if "?" in path else "?"
    for page in range(1, 101):
        data = gh_api(f"{path}{separator}per_page=100&page={page}")
        batch = data[key]
        result.extend(batch)
        if len(batch) < 100:
            return result
    raise ReleaseError("API pagination limit exceeded; refusing incomplete evidence")


def identity(sha, version, channel, confirmation):
    require(re.fullmatch(r"[0-9a-f]{40}", sha), "source_sha must be a full lowercase 40-character commit SHA")
    number = r"(?:0|[1-9][0-9]*)"
    base = rf"{number}\.{number}\.{number}"
    patterns = {"stable": base, "rc": rf"{base}-rc\.[1-9][0-9]*", "preview": rf"{base}-preview\.[1-9][0-9]*"}
    require(channel in patterns, "channel must be explicitly preview, rc, or stable")
    require(re.fullmatch(patterns[channel], version), f"version does not match {channel} channel policy")
    tag = f"v{version}"
    require(confirmation == f"release {channel} {tag} {sha}", "confirmation must exactly identify channel, tag, and full SHA")
    return tag


def validate_source(source, sha, version):
    require(command("git", "rev-parse", "HEAD", cwd=source) == sha, "checkout does not equal approved source SHA")
    require(command("git", "cat-file", "-t", sha, cwd=source) == "commit", "source must be a commit")
    require(not command("git", "status", "--porcelain", "--untracked-files=no", cwd=source), "tracked source files changed")
    manifest = tomllib.loads(command("git", "show", f"{sha}:Cargo.toml", cwd=source))
    lock = tomllib.loads(command("git", "show", f"{sha}:Cargo.lock", cwd=source))
    require(manifest["package"]["name"] == "hermes", "unexpected root package")
    require(manifest["package"]["version"] == version, "Cargo.toml version must already be committed")
    packages = [p for p in lock["package"] if p["name"] == "hermes" and "source" not in p]
    require(len(packages) == 1 and packages[0]["version"] == version, "Cargo.lock root version must match committed Cargo.toml")
    toolchain = tomllib.loads(command("git", "show", f"{sha}:rust-toolchain.toml", cwd=source))
    require(toolchain["toolchain"]["channel"] == RUST_VERSION, f"committed Rust toolchain must be {RUST_VERSION}")


def validate_ci(runs, jobs, sha, repository, branch, workflow_id):
    candidates = [r for r in runs if r.get("head_sha") == sha and r.get("event") == "push"
                  and r.get("head_branch") == branch and r.get("workflow_id") == workflow_id
                  and r.get("head_repository", {}).get("full_name") == repository]
    require(candidates, "no default-branch push CI run for the exact source SHA")
    latest = max(candidates, key=lambda r: (r["id"], r["run_attempt"]))
    require(latest.get("status") == "completed" and latest.get("conclusion") == "success", "latest exact-commit CI run did not succeed")
    require(jobs, "CI job evidence is empty")
    for name in REQUIRED_JOBS:
        matches = [j for j in jobs if j.get("name") == name]
        require(len(matches) == 1, f"missing or duplicate required CI job: {name}")
    for job in jobs:
        require(job.get("head_sha") == sha and job.get("run_id") == latest["id"], "CI job belongs to another commit or run")
        require(job.get("status") == "completed" and job.get("conclusion") == "success", f"CI job not successful: {job.get('name')}")
    return latest


def validate_environment(environment, approvals=None, actors=()):
    require(environment.get("name") == "release", "expected release environment")
    rules = [r for r in environment.get("protection_rules", []) if r.get("type") == "required_reviewers"]
    require(len(rules) == 1 and rules[0].get("reviewers"), "release environment must have required reviewers configured")
    if approvals is None:
        return
    relevant = [a for a in approvals if any(e.get("id") == environment["id"] for e in a.get("environments", []))]
    require(not any(a.get("state") == "rejected" for a in relevant), "release approval was rejected; start a new dispatch")
    approved = [a for a in relevant if a.get("state") == "approved"
                and a.get("user", {}).get("type") == "User"]
    require(approved, "no human approval recorded for this run's release environment")


def validate_remote(args):
    repository = os.environ["GITHUB_REPOSITORY"]
    require(re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository), "invalid repository")
    require(os.environ.get("GITHUB_EVENT_NAME") == "workflow_dispatch", "release requires manual dispatch")
    require(os.environ.get("GITHUB_RUN_ATTEMPT") == "1", "reruns are forbidden; start a fresh dispatch and approval")
    repo = gh_api(f"repos/{repository}")
    branch = repo["default_branch"]
    require(os.environ["GITHUB_REF"] == f"refs/heads/{branch}", "dispatch workflow must run from the default branch")
    command("git", "check-ref-format", f"refs/heads/{branch}", cwd=args.source)
    command("git", "fetch", "--no-tags", "origin", f"+refs/heads/{branch}:refs/remotes/origin/{branch}", cwd=args.source)
    command("git", "merge-base", "--is-ancestor", args.sha, f"refs/remotes/origin/{branch}", cwd=args.source)
    # Never accept a tag whose name collides, even if it already points to this SHA.
    require(not command("git", "ls-remote", "--tags", "origin", f"refs/tags/v{args.version}", cwd=args.source), "release tag already exists; tags are never moved")
    workflow = gh_api(f"repos/{repository}/actions/workflows/ci.yml")
    require(workflow.get("path") == ".github/workflows/ci.yml" and workflow.get("state") == "active", "required CI workflow is not active")
    query = urlencode({"head_sha": args.sha, "event": "push", "branch": branch})
    runs = pages(f"repos/{repository}/actions/workflows/{workflow['id']}/runs?{query}", "workflow_runs")
    candidates = [r for r in runs if r.get("head_sha") == args.sha and r.get("event") == "push"
                  and r.get("head_branch") == branch and r.get("workflow_id") == workflow["id"]
                  and r.get("head_repository", {}).get("full_name") == repository]
    require(candidates, "no required exact-commit CI run")
    latest = max(candidates, key=lambda r: (r["id"], r["run_attempt"]))
    jobs = pages(f"repos/{repository}/actions/runs/{latest['id']}/attempts/{latest['run_attempt']}/jobs", "jobs")
    ci = validate_ci(runs, jobs, args.sha, repository, branch, workflow["id"])
    if args.ci_run:
        require(args.ci_run == f"{ci['id']}:{ci['run_attempt']}", "CI run changed since preparation; start a fresh dispatch")
    environment = gh_api(f"repos/{repository}/environments/release")
    approvals = gh_api(f"repos/{repository}/actions/runs/{os.environ['GITHUB_RUN_ID']}/approvals") if args.require_approval else None
    validate_environment(environment, approvals, (os.environ.get("GITHUB_ACTOR"), os.environ.get("GITHUB_TRIGGERING_ACTOR")))
    return {"repository": repository, "default_branch": branch, "source_sha": args.sha,
            "version": args.version, "channel": args.channel, "tag": f"v{args.version}",
            "workflow_sha": os.environ["GITHUB_SHA"], "run_id": os.environ["GITHUB_RUN_ID"],
            "ci_run": f"{ci['id']}:{ci['run_attempt']}", "required_jobs": list(REQUIRED_JOBS),
            "successful_jobs": [j["name"] for j in jobs], "environment": "release",
            "approval_verified": args.require_approval}


def publish_commands(source, sha, version, channel, dist):
    tag = f"v{version}"
    assets = sorted(str(p.resolve()) for p in Path(dist).iterdir())
    require(assets, "no release assets")
    prerelease = ["--prerelease"] if channel != "stable" else []
    return [
        ["git", "-C", str(source), "-c", "user.name=github-actions[bot]", "-c", "user.email=41898282+github-actions[bot]@users.noreply.github.com", "tag", "-a", tag, sha, "-m", f"Hermes {tag} ({channel}) source {sha}"],
        ["git", "-C", str(source), "push", "origin", f"refs/tags/{tag}:refs/tags/{tag}"],
        ["gh", "release", "create", tag, *assets, "--verify-tag", "--draft", "--generate-notes", "--title", f"Hermes {tag}", "--latest=false", *prerelease],
        ["gh", "release", "edit", tag, "--draft=false", f"--latest={'true' if channel == 'stable' else 'false'}", f"--prerelease={'false' if channel == 'stable' else 'true'}"],
    ]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("source", "gate", "publish"))
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--sha", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--channel", required=True)
    parser.add_argument("--confirmation", required=True)
    parser.add_argument("--ci-run")
    parser.add_argument("--require-approval", action="store_true")
    parser.add_argument("--evidence", type=Path)
    parser.add_argument("--dist", type=Path)
    args = parser.parse_args()
    try:
        tag = identity(args.sha, args.version, args.channel, args.confirmation)
        validate_source(args.source, args.sha, args.version)
        if args.operation == "source":
            return
        if args.operation == "publish":
            require(args.require_approval and args.ci_run and args.dist, "publish requires approval, pinned CI run, and sealed assets")
            from release_artifacts import verify_seal
            verify_seal(args.dist, args.sha, args.version, args.channel)
        evidence = validate_remote(args)
        if args.evidence:
            args.evidence.write_text(json.dumps(evidence, indent=2) + "\n")
        if args.operation == "gate" and os.environ.get("GITHUB_OUTPUT"):
            with open(os.environ["GITHUB_OUTPUT"], "a") as output:
                for key, value in (("source_sha", args.sha), ("version", args.version), ("channel", args.channel), ("tag", tag), ("ci_run", evidence["ci_run"])):
                    output.write(f"{key}={value}\n")
        if args.operation == "publish":
            commands = publish_commands(args.source, args.sha, args.version, args.channel, args.dist)
            for index, cmd in enumerate(commands):
                subprocess.run(cmd, check=True)
                if index == 1:
                    remote = command("git", "ls-remote", "--tags", "origin", f"refs/tags/{tag}^{{}}", cwd=args.source)
                    require(remote.split() == [args.sha, f"refs/tags/{tag}^{{}}"], "remote tag does not resolve to approved SHA")
    except (ReleaseError, subprocess.CalledProcessError, KeyError, OSError, tomllib.TOMLDecodeError) as error:
        sys.exit(f"release refused: {error}")


if __name__ == "__main__":
    main()
