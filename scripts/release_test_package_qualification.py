"""Offline checks for qualification safety; never invokes host pacman."""

import hashlib
import json
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import sys
import unittest
from unittest.mock import patch

import qualify_arch_package as qualification


class QualificationSafetyTests(unittest.TestCase):
    def test_container_doc_policy_fix_preserves_other_pacman_policy(self):
        workflow = Path(__file__).resolve().parents[1] / ".github/workflows/packages.yml"
        command = next(line.strip() for line in workflow.read_text().splitlines() if "sed -i " in line)
        with tempfile.TemporaryDirectory(prefix="hermes-pacman-policy-") as temporary:
            config = Path(temporary) / "pacman.conf"
            retained = "[options]\nSigLevel = Required DatabaseOptional\nNoExtract = usr/share/man/*\n"
            config.write_text(retained + "NoExtract  = usr/share/gtk-doc/html/* usr/share/doc/*\n")
            arguments = shlex.split(command)
            arguments[-1] = str(config)
            subprocess.run(arguments, check=True)
            self.assertEqual(config.read_text(), retained)

    def test_refuses_normal_host_before_running_commands(self):
        with patch.object(os, "geteuid", return_value=1000), self.assertRaises(ValueError):
            qualification.require_disposable_root()

    def test_root_without_marker_is_refused(self):
        with patch.object(os, "geteuid", return_value=0), \
             patch.object(Path, "is_file", return_value=False), self.assertRaises(ValueError):
            qualification.require_disposable_root()

    def test_marker_does_not_authorize_host_root(self):
        contents = {"/.hermes-disposable-root": qualification.MARKER,
                    "/etc/os-release": "ID=arch\n",
                    "/proc/self/uid_map": "0 0 4294967295\n"}
        with patch.object(os, "geteuid", return_value=0), \
             patch.object(Path, "is_file", side_effect=lambda path: str(path) in contents, autospec=True), \
             patch.object(Path, "read_text", side_effect=lambda path: contents[str(path)], autospec=True), \
             self.assertRaises(ValueError):
            qualification.require_disposable_root()

    def test_multiple_private_uid_ranges_are_supported(self):
        contents = {"/.hermes-disposable-root": qualification.MARKER,
                    "/etc/os-release": "ID=arch\n",
                    "/proc/self/uid_map": "0 1000 1\n1 100000 65536\n"}
        with patch.object(os, "geteuid", return_value=0), \
             patch.object(Path, "is_file", return_value=True), \
             patch.object(Path, "read_text", side_effect=lambda path: contents[str(path)], autospec=True):
            qualification.require_disposable_root()


class CandidateTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="hermes-package-metadata-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.package = self.root / "hermes-file-manager-0.2.0-1-x86_64.pkg.tar.zst"
        self.package.write_bytes(b"fixture")
        self.sha = "a" * 40
        self.digest = hashlib.sha256(b"fixture").hexdigest()
        (self.root / "BUILD-INFO.json").write_text(json.dumps({"source_commit": self.sha}))
        (self.root / "SHA256SUMS").write_text(f"{self.digest}  {self.package.name}\n")

    def test_exact_identity_and_checksum(self):
        self.assertEqual(qualification.candidate_at(self.root), (self.package, self.sha, self.digest))

    def test_tampering_fails_before_installation(self):
        self.package.write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError, "checksum mismatch"):
            qualification.candidate_at(self.root)

    def test_traversal_and_ambiguous_artifacts_are_refused(self):
        for record in (f"{self.digest}  ../{self.package.name}\n", "",
                       f"{self.digest}  {self.package.name}\n" * 2):
            with self.subTest(record=record), self.assertRaises(ValueError):
                (self.root / "SHA256SUMS").write_text(record)
                qualification.candidate_at(self.root)

    def test_short_commit_is_refused(self):
        (self.root / "BUILD-INFO.json").write_text(json.dumps({"source_commit": "main"}))
        with self.assertRaises(ValueError):
            qualification.candidate_at(self.root)

    def test_other_source_commit_is_refused_before_package_commands(self):
        result = qualification.Qualification(self.root)
        with patch.object(qualification, "verify") as verify, self.assertRaisesRegex(ValueError, "source"):
            result.exercise(self.root, "b" * 40)
        verify.assert_not_called()

    def test_retention_detects_changed_removed_and_symlinked_data(self):
        expected = qualification.fixtures(self.root / "profile")
        qualification.assert_retained(expected)
        target, original = next(iter(expected.items()))
        target.write_bytes(b"changed")
        with self.assertRaises(ValueError):
            qualification.assert_retained(expected)
        target.unlink()
        with self.assertRaises(ValueError):
            qualification.assert_retained(expected)
        other = self.root / "other"
        other.write_bytes(original)
        target.symlink_to(other)
        with self.assertRaises(ValueError):
            qualification.assert_retained(expected)

    def test_report_keeps_unverified_release_gates_pending(self):
        result = qualification.Qualification(self.root)
        result.passed("same-version reinstall retention")
        report = json.loads((self.root / "RESULTS.json").read_text())
        self.assertIn("previous-release upgrade", report["pending"])
        self.assertIn("interactive migration and recovery", report["pending"])

    def test_failed_commands_keep_their_diagnostic_log(self):
        result = qualification.Qualification(self.root)
        with self.assertRaisesRegex(RuntimeError, "probe failed"):
            result.run("probe", [sys.executable, "-c", "print('failure evidence'); raise SystemExit(2)"])
        self.assertEqual((self.root / "probe.log").read_text(), "failure evidence\n")


class UpgradeBaselineTests(unittest.TestCase):
    def setUp(self):
        self.package = Path("/fixtures/old.pkg.tar.zst")
        self.sha = "a" * 40
        self.digest = "b" * 64
        patch.object(qualification, "candidate_at", return_value=(self.package, self.sha, self.digest)).start()
        self.verify = patch.object(qualification, "verify").start()
        self.command = patch.object(qualification.subprocess, "check_output").start()
        self.addCleanup(patch.stopall)

    def check(self, previous="0.2.0-1", current="0.2.0-2", comparison="-1", previous_arch="x86_64"):
        self.command.side_effect = [f"pkgname = hermes-file-manager\npkgver = {previous}\narch = {previous_arch}\n",
                                    f"pkgname = hermes-file-manager\npkgver = {current}\narch = x86_64\n", comparison]
        return qualification.upgrade_baseline(Path("/fixtures/baseline"), self.sha, Path("/fixtures/candidate.pkg.tar.zst"))

    def test_older_immutable_baseline_keeps_both_identities(self):
        package, info = self.check()
        self.assertEqual(package, self.package)
        self.assertEqual(info, {"source_commit": self.sha, "package_sha256": self.digest,
                                "previous_version": "0.2.0-1", "candidate_version": "0.2.0-2"})
        self.command.assert_called_with(["vercmp", "0.2.0-1", "0.2.0-2"], text=True)

    def test_equal_version_and_downgrade_are_rejected(self):
        for comparison in ("0", "1"):
            with self.subTest(comparison=comparison), self.assertRaisesRegex(ValueError, "strictly older"):
                self.check(comparison=comparison)

    def test_architecture_mismatch_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "architectures"):
            self.check(previous_arch="aarch64")

    def test_other_source_is_rejected_before_metadata_commands(self):
        with self.assertRaisesRegex(ValueError, "Baseline source"):
            qualification.upgrade_baseline(Path("/fixtures/baseline"), "c" * 40, Path("/fixtures/new"))
        self.command.assert_not_called()
        self.verify.assert_not_called()

    def test_foreign_package_name_is_rejected(self):
        self.command.return_value = "pkgname = another-app\npkgver = 0.1.0-1\narch = x86_64\n"
        with self.assertRaisesRegex(ValueError, "package identities"):
            qualification.upgrade_baseline(Path("/fixtures/baseline"), self.sha, Path("/fixtures/new"))


if __name__ == "__main__":
    unittest.main()
