"""Run locked test artifacts with private GTK/D-Bus/XDG state and record exclusions."""

import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[1]


def inventory(binary, ignored=False):
    args = [str(binary), "--list", "--format", "terse"]
    if ignored:
        args.append("--ignored")
    output = subprocess.check_output(args, cwd=ROOT, text=True)
    return [line.removesuffix(": test") for line in output.splitlines() if line.endswith(": test")]


def reasons():
    result = {}
    for source in list((ROOT / "src").rglob("*.rs")) + list((ROOT / "tests").rglob("*.rs")):
        text = source.read_text()
        for match in re.finditer(r'#\[ignore\s*=\s*"([^"\n]+)"\](?:(?!\bfn\b).)*?\bfn\s+(\w+)', text, re.S):
            reason, function = match.groups()
            result.setdefault(function, []).append({"source": str(source.relative_to(ROOT)), "reason": reason})
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--jobs", type=int, default=2, help="Cargo build jobs; tests use one thread")
    args = parser.parse_args()
    for tool in ("gtk4-broadwayd", "dbus-run-session", "bwrap"):
        if not shutil.which(tool):
            parser.error(f"Missing test dependency: {tool}")
    base = Path(tempfile.mkdtemp(prefix="hermes-ci-tests-"))
    print(f"Private test logs/profiles and coverage manifest: {base}", flush=True)
    spec = importlib.util.spec_from_file_location("safety", ROOT / "scripts/check-release-safety.py")
    safety = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(safety)
    # Build the worker too: generated integration tests find this sibling binary.
    subprocess.run(["cargo", "build", "--locked", "--bin", "hermes", "-j", str(args.jobs)], cwd=ROOT, check=True)
    build = subprocess.run(["cargo", "test", "--locked", "--all-targets", "--all-features", "--no-run",
                            "--message-format=json", "-j", str(args.jobs)], cwd=ROOT, capture_output=True, text=True)
    (base / "build.log").write_text(build.stderr)
    artifacts = []
    for line in build.stdout.splitlines():
        event = json.loads(line)
        if event.get("reason") == "compiler-message":
            with (base / "build.log").open("a") as stream:
                stream.write(event["message"].get("rendered") or event["message"]["message"])
        if event.get("reason") == "compiler-artifact" and event.get("profile", {}).get("test") and event.get("executable"):
            artifacts.append((event["target"]["name"], Path(event["executable"])))
    if build.returncode or not artifacts:
        raise SystemExit(f"Test compilation failed; see {base / 'build.log'}")
    ignored_reasons = reasons()
    coverage = {"artifacts": [], "executed_nonignored": [], "executed_isolated": [], "excluded": [], "physical_acceptance": "pending", "status": "running"}
    def checkpoint():
        (base / "coverage.json").write_text(json.dumps(coverage, indent=2) + "\n")

    checkpoint()
    case_index = 0
    unit = None
    for target, binary in artifacts:
        all_tests = inventory(binary)
        ignored = inventory(binary, True)
        if not all_tests:
            continue
        coverage["artifacts"].append({"target": target, "tests": len(all_tests), "ignored": len(ignored)})
        # The harness accepts a substring filter; empty matches every nonignored test.
        coverage["current_case"] = {"target": target, "kind": "nonignored"}
        checkpoint()
        try:
            safety.run_case(binary, base, "", case_index, False)
        except Exception:
            coverage["status"] = "failed"
            checkpoint()
            raise
        coverage["executed_nonignored"].append({"target": target, "count": len(all_tests) - len(ignored)})
        case_index += 1
        if target == "hermes":
            unit = binary
        for name in ignored:
            reason = ignored_reasons.get(name.rsplit("::", 1)[-1])
            if not reason:
                raise SystemExit(f"Ignored test lacks an explicit dependency/platform reason: {target}::{name}")
            coverage["excluded"].append({"target": target, "name": name, "requirements": reason})
    if unit is None:
        raise SystemExit("The Hermes unit-test artifact is missing")
    ignored = inventory(unit, True)
    required = set(safety.GTK_TESTS)
    # Exercise generated, non-decoder GTK acceptance cases. Other ignored tests
    # retain their precise dependency/corpus reasons in the persisted manifest.
    for name in ignored:
        requirement = " ".join(item["reason"].lower() for item in ignored_reasons[name.rsplit("::", 1)[-1]])
        if name.startswith("ui::") and any(word in requirement for word in ("isolated gtk", "private gtk")):
            if not any(word in requirement for word in ("ffmpeg", "worker", "audio backend", "renderer", "personal media", "home/xdg")):
                required.add(name)
    try:
        for name in sorted(required):
            coverage["current_case"] = {"target": "hermes", "name": name, "kind": "isolated"}
            checkpoint()
            safety.run_case(unit, base, name, case_index, True)
            case_index += 1
            coverage["executed_isolated"].append(name)
            coverage["excluded"] = [e for e in coverage["excluded"] if not (e["target"] == "hermes" and e["name"] == name)]
        coverage["status"] = "passed"
        coverage.pop("current_case", None)
    except Exception:
        coverage["status"] = "failed"
        raise
    finally:
        checkpoint()
    print(f"All nonignored test artifacts and {len(required)} isolated GTK cases passed; {len(coverage['excluded'])} dependency/corpus checks remain explicitly recorded.")


if __name__ == "__main__":
    main()
