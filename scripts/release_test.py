#!/usr/bin/env python3
"""Offline release gate tests. Fixtures stay inside this checkout; no remote writes."""

import argparse
import copy
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import release
import release_artifacts as artifacts

ROOT = Path(__file__).resolve().parents[1]
SHA = "a" * 40
OTHER = "b" * 40
REPOSITORY = "example/hermes"
ENV = {"GITHUB_SHA": OTHER, "GITHUB_RUN_ID": "123", "GITHUB_RUN_ATTEMPT": "1",
       "GITHUB_REPOSITORY": REPOSITORY, "GITHUB_EVENT_NAME": "workflow_dispatch",
       "GITHUB_REF": "refs/heads/main", "GITHUB_ACTOR": "author", "GITHUB_TRIGGERING_ACTOR": "author"}


def run_record(**changes):
    result = {"id": 42, "run_attempt": 1, "head_sha": SHA, "event": "push", "head_branch": "main",
              "workflow_id": 7, "head_repository": {"full_name": REPOSITORY},
              "status": "completed", "conclusion": "success"}
    result.update(changes)
    return result


def jobs():
    return [{"name": name, "head_sha": SHA, "run_id": 42, "status": "completed", "conclusion": "success"}
            for name in release.REQUIRED_JOBS]


def environment():
    return {"id": 99, "name": "release", "protection_rules": [{"type": "required_reviewers",
            "prevent_self_review": True, "reviewers": [{"type": "User", "reviewer": {"login": "reviewer"}}]}]}


def approvals():
    return [{"state": "approved", "environments": [{"id": 99, "name": "release"}],
             "user": {"login": "reviewer", "type": "User"}}]


class IdentityTests(unittest.TestCase):
    def check(self, version="1.2.3", channel="stable", sha=SHA, confirmation=None):
        return release.identity(sha, version, channel, confirmation or f"release {channel} v{version} {sha}")

    def test_deliberate_channels(self):
        for version, channel in (("1.2.3", "stable"), ("1.2.3-rc.1", "rc"), ("1.2.3-preview.9", "preview")):
            with self.subTest(channel=channel):
                self.assertEqual(self.check(version, channel), f"v{version}")

    def test_reject_bad_or_ambiguous_versions(self):
        for version in ("v1.2.3", "1.2", "01.2.3", "1.2.3+build", "1.2.3\nmalicious", "1.2.3-rc.01", "1.2.3-rc.0", "1.2.3-preview.1"):
            with self.subTest(version=version), self.assertRaises(release.ReleaseError):
                self.check(version)

    def test_no_channel_inference_or_promotion(self):
        for version, channel in (("1.2.3", "rc"), ("1.2.3-rc.1", "stable"), ("1.2.3-rc.1", "preview"), ("1.2.3", "")):
            with self.subTest(channel=channel), self.assertRaises(release.ReleaseError):
                self.check(version, channel)

    def test_full_sha_only(self):
        for sha in (SHA[:12], SHA.upper(), "main", "HEAD", SHA + "\n", "-" * 40):
            with self.subTest(sha=sha), self.assertRaises(release.ReleaseError):
                self.check(sha=sha)

    def test_confirmation_binds_all_inputs(self):
        for confirmation in ("yes", f"release stable v1.2.3 {OTHER}", f"release rc v1.2.3 {SHA}"):
            with self.assertRaises(release.ReleaseError):
                self.check(confirmation=confirmation)


class CITests(unittest.TestCase):
    def check(self, runs=None, job_list=None):
        return release.validate_ci(runs if runs is not None else [run_record()], jobs() if job_list is None else job_list, SHA, REPOSITORY, "main", 7)

    def test_exact_run_and_all_jobs_succeed(self):
        self.assertEqual(self.check()["id"], 42)

    def test_reject_wrong_sha_branch_event_repo_workflow(self):
        for changes in ({"head_sha": OTHER}, {"head_branch": "feature"}, {"event": "pull_request"},
                        {"head_repository": {"full_name": "fork/hermes"}}, {"workflow_id": 8}):
            with self.subTest(changes=changes), self.assertRaises(release.ReleaseError):
                self.check([run_record(**changes)])

    def test_no_fallback_to_older_success(self):
        for state in ("failure", "cancelled", "skipped", "neutral", None):
            with self.subTest(state=state), self.assertRaises(release.ReleaseError):
                self.check([run_record(id=41), run_record(conclusion=state)])

    def test_rerun_must_itself_succeed(self):
        with self.assertRaises(release.ReleaseError):
            self.check([run_record(), run_record(run_attempt=2, status="in_progress", conclusion=None)])

    def test_missing_duplicate_skipped_or_wrong_commit_jobs_fail(self):
        variants = [[], jobs()[:-1], jobs() + [jobs()[0]]]
        for changes in ({"head_sha": OTHER}, {"run_id": 43}, {"status": "in_progress"}, {"conclusion": "skipped"}):
            changed = jobs()
            changed[0].update(changes)
            variants.append(changed)
        for variant in variants:
            with self.subTest(jobs=variant), self.assertRaises(release.ReleaseError):
                self.check(job_list=variant)

    def test_additional_native_package_jobs_cannot_be_skipped(self):
        extra = {**jobs()[0], "name": "Arch package checks", "conclusion": "skipped"}
        with self.assertRaises(release.ReleaseError):
            self.check(job_list=jobs() + [extra])
        extra["conclusion"] = "success"
        self.check(job_list=jobs() + [extra])


class ApprovalTests(unittest.TestCase):
    def test_protected_environment_and_independent_approval(self):
        release.validate_environment(environment(), approvals(), ("author",))

    def test_unprotected_environment_fails(self):
        variants = [{"name": "release"}, {**environment(), "name": "other"}]
        for change in ({"reviewers": []},):
            value = environment()
            value["protection_rules"][0].update(change)
            variants.append(value)
        for value in variants:
            with self.assertRaises(release.ReleaseError):
                release.validate_environment(value)

    def test_approval_must_be_human_for_this_environment(self):
        variants = [[]]
        for change in ({"state": "rejected"}, {"environments": [{"id": 100}]},
                       {"user": {"login": "bot", "type": "Bot"}}):
            value = approvals()
            value[0].update(change)
            variants.append(value)
        for value in variants:
            with self.assertRaises(release.ReleaseError):
                release.validate_environment(environment(), value, ("author",))

    def test_project_owner_can_explicitly_approve_their_release(self):
        owner_approval = approvals()
        owner_approval[0]["user"] = {"login": "author", "type": "User"}
        release.validate_environment(environment(), owner_approval, ("author",))

    def test_rejected_run_requires_fresh_dispatch(self):
        rejected = {**approvals()[0], "state": "rejected"}
        with self.assertRaises(release.ReleaseError):
            release.validate_environment(environment(), approvals() + [rejected], ("author",))


class SourceTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix=".release-tests-", dir=ROOT)
        self.addCleanup(self.temporary.cleanup)
        self.source = Path(self.temporary.name)
        self.git("init", "--initial-branch=main")
        self.git("config", "user.name", "Release test")
        self.git("config", "user.email", "release-test@example.invalid")
        (self.source / "Cargo.toml").write_text('[package]\nname = "hermes"\nversion = "1.2.3"\n')
        (self.source / "Cargo.lock").write_text('version = 4\n[[package]]\nname = "hermes"\nversion = "1.2.3"\n')
        (self.source / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "1.99.0"\n')
        self.commit()

    def git(self, *args):
        return subprocess.check_output(["git", "-c", "commit.gpgsign=false", "-C", str(self.source), *args], text=True, stderr=subprocess.PIPE).strip()

    def commit(self):
        self.git("add", ".")
        self.git("commit", "-m", "fixture")
        self.sha = self.git("rev-parse", "HEAD")

    def test_committed_matching_versions_pass_without_modification(self):
        release.validate_source(self.source, self.sha, "1.2.3")
        self.assertEqual(self.git("status", "--porcelain"), "")

    def test_uncommitted_and_staged_changes_fail(self):
        (self.source / "Cargo.toml").write_text("changed")
        with self.assertRaises(release.ReleaseError):
            release.validate_source(self.source, self.sha, "1.2.3")
        self.git("add", ".")
        with self.assertRaises(release.ReleaseError):
            release.validate_source(self.source, self.sha, "1.2.3")

    def test_version_is_never_bumped(self):
        with self.assertRaises(release.ReleaseError):
            release.validate_source(self.source, self.sha, "1.2.4")
        self.assertEqual(self.git("status", "--porcelain"), "")

    def test_mismatching_committed_lock_fails(self):
        (self.source / "Cargo.lock").write_text('[[package]]\nname = "hermes"\nversion = "1.2.2"\n')
        self.commit()
        with self.assertRaises(release.ReleaseError):
            release.validate_source(self.source, self.sha, "1.2.3")

    def test_moving_checkout_and_toolchain_drift_fail(self):
        with self.assertRaises(release.ReleaseError):
            release.validate_source(self.source, OTHER, "1.2.3")
        (self.source / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "stable"\n')
        self.commit()
        with self.assertRaises(release.ReleaseError):
            release.validate_source(self.source, self.sha, "1.2.3")

    def test_source_archive_is_committed_bytes_and_repeatable(self):
        one, two = self.source / "one.tar.gz", self.source / "two.tar.gz"
        artifacts.source_archive(self.source, self.sha, "1.2.3", one)
        artifacts.source_archive(self.source, self.sha, "1.2.3", two)
        self.assertEqual(one.read_bytes(), two.read_bytes())
        with tarfile.open(one) as archive:
            self.assertEqual(archive.extractfile("hermes-1.2.3/Cargo.toml").read(), (self.source / "Cargo.toml").read_bytes())
        with self.assertRaises(release.ReleaseError):
            artifacts.source_archive(self.source, self.sha, "1.2.3", one)


class RemoteGateTests(unittest.TestCase):
    def setUp(self):
        self.args = argparse.Namespace(source=Path("source"), sha=SHA, version="1.2.3", channel="stable", ci_run="42:1", require_approval=True)
        self.environ = patch.dict(os.environ, ENV)
        self.environ.start()
        self.addCleanup(self.environ.stop)
        self.command = patch.object(release, "command", return_value="").start()
        self.addCleanup(patch.stopall)
        self.api = patch.object(release, "gh_api", side_effect=self.api_response).start()
        self.paginate = patch.object(release, "pages", side_effect=lambda path, key: [run_record()] if key == "workflow_runs" else jobs()).start()

    def api_response(self, path):
        if path == f"repos/{REPOSITORY}":
            return {"default_branch": "main"}
        if path.endswith("/workflows/ci.yml"):
            return {"id": 7, "path": ".github/workflows/ci.yml", "state": "active"}
        if path.endswith("/environments/release"):
            return environment()
        if path.endswith("/approvals"):
            return approvals()
        self.fail(f"unexpected API call: {path}")

    def test_remote_gate_fetches_default_branch_checks_ancestry_and_latest_attempt(self):
        evidence = release.validate_remote(self.args)
        self.assertTrue(evidence["approval_verified"])
        self.command.assert_any_call("git", "merge-base", "--is-ancestor", SHA, "refs/remotes/origin/main", cwd=Path("source"))
        self.paginate.assert_any_call(f"repos/{REPOSITORY}/actions/runs/42/attempts/1/jobs", "jobs")

    def test_wrong_dispatch_ref_or_event_or_rerun_fails(self):
        for key, value in (("GITHUB_REF", "refs/heads/feature"), ("GITHUB_REF", "refs/tags/main"),
                           ("GITHUB_EVENT_NAME", "push"), ("GITHUB_RUN_ATTEMPT", "2")):
            with patch.dict(os.environ, {key: value}), self.assertRaises(release.ReleaseError):
                release.validate_remote(self.args)

    def test_unreachable_source_fails_closed(self):
        def fake_command(*args, **kwargs):
            if "merge-base" in args:
                raise subprocess.CalledProcessError(1, args)
            return ""
        self.command.side_effect = fake_command
        with self.assertRaises(subprocess.CalledProcessError):
            release.validate_remote(self.args)

    def test_existing_remote_tag_fails_even_if_correct_sha(self):
        self.command.side_effect = lambda *args, **kwargs: f"{SHA}\trefs/tags/v1.2.3" if "ls-remote" in args else ""
        with self.assertRaises(release.ReleaseError):
            release.validate_remote(self.args)

    def test_changed_ci_run_is_not_implicitly_approved(self):
        self.args.ci_run = "41:1"
        with self.assertRaises(release.ReleaseError):
            release.validate_remote(self.args)

    def test_api_failure_is_not_a_waiver(self):
        self.api.side_effect = subprocess.CalledProcessError(1, ["gh", "api"])
        with self.assertRaises(subprocess.CalledProcessError):
            release.validate_remote(self.args)


class ArtifactTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix=".release-tests-", dir=ROOT)
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.dist = self.root / "dist"
        self.dist.mkdir()
        self.environ = patch.dict(os.environ, ENV)
        self.environ.start()
        self.addCleanup(self.environ.stop)

    def sealed_fixture(self):
        payload = self.dist / "source.tar.gz"
        payload.write_bytes(b"committed source fixture")
        manifest = {"schema_version": 1, "approval_verified": True, "source_sha": SHA, "version": "1.2.3",
                    "channel": "stable", "rust": release.RUST_VERSION, "workflow_sha": OTHER, "run_id": "123",
                    "assets": {payload.name: artifacts.digest(payload)}}
        artifacts.write_json(self.dist / "release-manifest.json", manifest)
        hashes = {**manifest["assets"], "release-manifest.json": artifacts.digest(self.dist / "release-manifest.json")}
        (self.dist / "SHA256SUMS").write_text("".join(f"{value}  {name}\n" for name, value in sorted(hashes.items())))

    def test_seal_verifies_and_detects_corruption(self):
        self.sealed_fixture()
        artifacts.verify_seal(self.dist, SHA, "1.2.3", "stable")
        (self.dist / "source.tar.gz").write_bytes(b"tampered")
        with self.assertRaises(release.ReleaseError):
            artifacts.verify_seal(self.dist, SHA, "1.2.3", "stable")

    def test_added_asset_channel_or_run_drift_fails(self):
        self.sealed_fixture()
        for args in ((OTHER, "1.2.3", "stable"), (SHA, "1.2.4", "stable"), (SHA, "1.2.3", "rc")):
            with self.assertRaises(release.ReleaseError):
                artifacts.verify_seal(self.dist, *args)
        with patch.dict(os.environ, {"GITHUB_RUN_ID": "124"}), self.assertRaises(release.ReleaseError):
            artifacts.verify_seal(self.dist, SHA, "1.2.3", "stable")
        (self.dist / "unexpected.deb").write_bytes(b"unapproved")
        with self.assertRaises(release.ReleaseError):
            artifacts.verify_seal(self.dist, SHA, "1.2.3", "stable")

    def test_asset_paths_and_symlinks_rejected(self):
        outside = self.root / "outside"
        outside.write_text("outside")
        (self.dist / "link").symlink_to(outside)
        for hashes in ({"../outside": artifacts.digest(outside)}, {"link": artifacts.digest(outside)}):
            with self.assertRaises(release.ReleaseError):
                artifacts.verify_files(self.dist, hashes)

    def test_license_inventory_uses_locked_graph_and_copies_texts(self):
        crate = self.root / "crate"
        crate.mkdir()
        (crate / "LICENSE").write_text("Fixture license")
        package = {"id": "crate-id", "name": "fixture", "version": "1.0.0", "source": "registry+fixture", "license": "MIT",
                   "license_file": "LICENSE", "manifest_path": str(crate / "Cargo.toml")}
        metadata = {"version": 1, "packages": [package], "resolve": {"nodes": [{"id": "crate-id"}]}}
        lock = {"package": [{"name": "fixture", "version": "1.0.0", "source": "registry+fixture", "checksum": "c" * 64}]}
        result = artifacts.dependency_inventory(metadata, lock, self.root / "dependency-licenses")
        self.assertEqual(result[0]["license"], "MIT")
        self.assertEqual(result[0]["checksum"], "c" * 64)
        self.assertEqual((self.root / result[0]["license_texts"][0]["path"]).read_text(), "Fixture license")
        with self.assertRaises(release.ReleaseError):
            artifacts.dependency_inventory(metadata, {"package": []}, self.root / "other")
        package["license_file"] = "../outside"
        (self.root / "outside").write_text("not a crate file")
        with self.assertRaises(release.ReleaseError):
            artifacts.dependency_inventory(metadata, lock, self.root / "other")

    def test_publish_commands_tag_exact_sha_and_never_push_a_branch(self):
        self.sealed_fixture()
        for channel in ("preview", "rc", "stable"):
            commands = release.publish_commands("source", SHA, "1.2.3", channel, self.dist)
            self.assertIn(SHA, commands[0])
            self.assertEqual(commands[1], ["git", "-C", "source", "push", "origin", "refs/tags/v1.2.3:refs/tags/v1.2.3"])
            self.assertIn("--verify-tag", commands[2])
            self.assertIn("--draft", commands[2])
            self.assertIn(f"--latest={'true' if channel == 'stable' else 'false'}", commands[3])
            self.assertEqual("--prerelease" in commands[2], channel != "stable")


if __name__ == "__main__":
    unittest.main()
