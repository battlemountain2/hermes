"""Offline stdlib tests using synthetic headers, never executing a native binary.

Run with python3 -m unittest discover -s scripts -p test_macos_bundle.py -v
"""

import hashlib
import json
import os
from pathlib import Path
import plistlib
import runpy
import shutil
import stat
import struct
import subprocess
import sys
import tempfile
import tomllib
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[1]
BUNDLER = ROOT / "scripts/build-macos-dev.py"
SHA = "0123456789abcdef0123456789abcdef01234567"
ARM64 = struct.pack("<8I", 0xFEEDFACF, 0x0100000C, 0, 2, 0, 0, 0, 0)
BUILD_BUNDLE = runpy.run_path(str(BUNDLER))["build_bundle"]


class MacOSBundleTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.source = self.root / "checkout"
        (self.source / "scripts").mkdir(parents=True)
        (self.source / "packaging/macos").mkdir(parents=True)
        shutil.copyfile(BUNDLER, self.source / "scripts/build-macos-dev.py")
        shutil.copyfile(ROOT / "Cargo.toml", self.source / "Cargo.toml")
        self.icon = self.source / "packaging/macos/Hermes.icns"
        shutil.copyfile(ROOT / "packaging/macos/Hermes.icns", self.icon)
        self.brewfile = self.source / "packaging/macos/Brewfile"
        self.brewfile.write_text('brew "gtk4"\n', encoding="utf-8")
        with (self.source / "Cargo.toml").open("rb") as manifest:
            self.version = tomllib.load(manifest)["package"]["version"]
        self.binary = self.root / "native binary"
        self.payload = ARM64 + b"\x00fixture bytes only\xff\n"
        self.binary.write_bytes(self.payload)
        self.binary.chmod(0o640)
        self.output = self.root / "test output"

    def bundle(self, **overrides):
        arguments = dict(binary=self.binary, output=self.output,
                         source_commit=SHA, source=self.source)
        arguments.update(overrides)
        BUILD_BUNDLE(**arguments)

    def cli(self, *extra):
        return subprocess.run(
            [sys.executable, str(self.source / "scripts/build-macos-dev.py"),
             "--binary", str(self.binary), "--output", str(self.output),
             "--source-commit", SHA, *extra],
            capture_output=True, text=True, check=False,
        )

    def assert_no_artifact(self):
        self.assertFalse(os.path.lexists(self.output))
        self.assertEqual(list(self.root.glob(".hermes-macos-dev-*")), [])

    def test_cli_bundle_structure_bytes_identity_and_version(self):
        result = self.cli()
        self.assertEqual(result.returncode, 0, result.stderr)
        contents = self.output / "Hermes Dev.app/Contents"
        expected = {"Hermes Dev.app/Contents/MacOS/hermes",
                    "Hermes Dev.app/Contents/Resources/Hermes.icns",
                    "Hermes Dev.app/Contents/Info.plist", "Brewfile", "README.txt",
                    "SOURCE-INFO.json", "BUILD-INFO.json"}
        self.assertEqual({str(path.relative_to(self.output))
                          for path in self.output.rglob("*") if path.is_file()}, expected)
        executable = contents / "MacOS/hermes"
        self.assertEqual(executable.read_bytes(), self.payload)
        self.assertEqual(stat.S_IMODE(executable.stat().st_mode), 0o755)
        self.assertEqual(self.binary.read_bytes(), self.payload)
        self.assertEqual(stat.S_IMODE(self.binary.stat().st_mode), 0o640)
        self.assertEqual((contents / "Resources/Hermes.icns").read_bytes(), self.icon.read_bytes())
        self.assertEqual((self.output / "Brewfile").read_bytes(), self.brewfile.read_bytes())
        with (contents / "Info.plist").open("rb") as stream:
            plist = plistlib.load(stream)
        self.assertEqual(plist, {
            "CFBundleExecutable": "hermes",
            "CFBundleIdentifier": "io.github.battlemountain2.Hermes.Dev",
            "CFBundleName": "Hermes Dev", "CFBundleDisplayName": "Hermes Dev",
            "CFBundlePackageType": "APPL", "CFBundleVersion": self.version,
            "CFBundleShortVersionString": self.version, "CFBundleIconFile": "Hermes.icns",
            "LSMinimumSystemVersion": "15.0", "NSHighResolutionCapable": True,
        })
        provenance = json.loads((self.output / "SOURCE-INFO.json").read_text())
        self.assertEqual(provenance, {"source_commit": SHA, "architecture": "arm64",
                                     "binary_sha256": hashlib.sha256(self.payload).hexdigest()})
        info = json.loads((self.output / "BUILD-INFO.json").read_text())
        self.assertEqual(info["version"], self.version)
        for key in ("dependency_versions", "sdk", "toolchain"):
            self.assertIsNone(info[key])
        self.assertEqual(list(self.root.glob(".hermes-macos-dev-*")), [])

    def test_readme_dependency_and_native_startup_instructions(self):
        self.bundle()
        readme = (self.output / "README.txt").read_text()
        for text in ("macOS 15.0 or later", "Apple Silicon", "dynamically linked Homebrew",
                     "brew bundle --file ./Brewfile", "--features macos-dev",
                     './"Hermes Dev.app"/Contents/MacOS/hermes --diagnostics',
                     'open "Hermes Dev.app"', "Dev roots", "no signing or notarization",
                     "native runtime acceptance"):
            self.assertIn(text, readme)

    def test_optional_deployment_target(self):
        result = self.cli("--deployment-target", "15.2")
        self.assertEqual(result.returncode, 0, result.stderr)
        with (self.output / "Hermes Dev.app/Contents/Info.plist").open("rb") as stream:
            self.assertEqual(plistlib.load(stream)["LSMinimumSystemVersion"], "15.2")
        self.assertIn("macOS 15.2 or later", (self.output / "README.txt").read_text())

    def test_version_tracks_cargo_manifest(self):
        (self.source / "Cargo.toml").write_text(
            '[package]\nname = "hermes"\nversion = "9.8.7"\n', encoding="utf-8")
        self.bundle()
        with (self.output / "Hermes Dev.app/Contents/Info.plist").open("rb") as stream:
            plist = plistlib.load(stream)
        self.assertEqual(plist["CFBundleVersion"], "9.8.7")
        self.assertEqual(plist["CFBundleShortVersionString"], "9.8.7")
        self.assertEqual(json.loads((self.output / "BUILD-INFO.json").read_text())
                         ["version"], "9.8.7")

    def test_reject_invalid_targets(self):
        for target in ("14.9", "10.15", "15", "15.x", "-15.0", "15.0\n", "15.0;echo"):
            with self.subTest(target=target), self.assertRaises(ValueError):
                self.bundle(deployment_target=target)
            self.assert_no_artifact()

    def test_source_sha_bounds(self):
        for sha in ("", "a" * 39, "a" * 41, "g" * 40, SHA + "\n", "../" + "a" * 37):
            with self.subTest(sha=sha), self.assertRaises(ValueError):
                self.bundle(source_commit=sha)
            self.assert_no_artifact()
        self.bundle(source_commit=SHA.upper())
        self.assertEqual(json.loads((self.output / "SOURCE-INFO.json").read_text())
                         ["source_commit"], SHA.upper())

    def test_reject_non_arm64_thin_executables(self):
        fixtures = {
            "elf": b"\x7fELF" + bytes(60), "empty": b"", "truncated": ARM64[:-1],
            "intel": struct.pack("<8I", 0xFEEDFACF, 0x01000007, 3, 2, 0, 0, 0, 0),
            "fat": struct.pack(">I", 0xCAFEBABE) + bytes(60),
            "fat64": struct.pack(">I", 0xCAFEBABF) + bytes(60),
            "big_endian": struct.pack(">8I", 0xFEEDFACF, 0x0100000C, 0, 2, 0, 0, 0, 0),
            "32_bit": struct.pack("<I", 0xFEEDFACE) + bytes(60),
            "dylib": struct.pack("<8I", 0xFEEDFACF, 0x0100000C, 0, 6, 0, 0, 0, 0),
        }
        for name, data in fixtures.items():
            self.binary.write_bytes(data)
            with self.subTest(name=name), self.assertRaises(ValueError):
                self.bundle()
            self.assert_no_artifact()

    def test_reject_symlink_binary_and_preserve_target(self):
        link = self.root / "binary link"
        link.symlink_to(self.binary)
        with self.assertRaises(ValueError):
            self.bundle(binary=link)
        self.assertEqual(self.binary.read_bytes(), self.payload)
        self.assert_no_artifact()

    def test_reject_missing_directory_and_fifo_binaries(self):
        fifo = self.root / "fifo"
        os.mkfifo(fifo)
        for binary in (self.root / "missing", self.source, fifo):
            with self.subTest(binary=binary), self.assertRaises((OSError, ValueError)):
                self.bundle(binary=binary)
            self.assert_no_artifact()

    def test_existing_bundle_and_file_are_preserved(self):
        self.bundle()
        snapshot = {path.relative_to(self.output): path.read_bytes()
                    for path in self.output.rglob("*") if path.is_file()}
        with self.assertRaises(FileExistsError):
            self.bundle()
        self.assertEqual(snapshot, {path.relative_to(self.output): path.read_bytes()
                                    for path in self.output.rglob("*") if path.is_file()})
        shutil.rmtree(self.output)
        self.output.write_bytes(b"existing artifact")
        with self.assertRaises(FileExistsError):
            self.bundle()
        self.assertEqual(self.output.read_bytes(), b"existing artifact")

    def test_existing_empty_directory_and_dangling_symlink_are_preserved(self):
        self.output.mkdir()
        with self.assertRaises(FileExistsError):
            self.bundle()
        self.assertEqual(list(self.output.iterdir()), [])
        self.output.rmdir()
        self.output.symlink_to(self.root / "absent target")
        with self.assertRaises(FileExistsError):
            self.bundle()
        self.assertTrue(self.output.is_symlink())
        self.assertFalse(self.output.exists())

    def test_staging_failure_cleans_up_without_publishing(self):
        for asset in (self.icon, self.brewfile):
            original = asset.read_bytes()
            asset.unlink()
            with self.subTest(asset=asset), self.assertRaises(FileNotFoundError):
                self.bundle()
            self.assert_no_artifact()
            self.assertEqual(self.binary.read_bytes(), self.payload)
            asset.write_bytes(original)

    def test_atomic_publication_never_replaces_concurrent_output(self):
        namespace = BUILD_BUNDLE.__globals__
        rename = namespace["rename_exclusive"]

        def competing_writer(candidate, destination):
            self.assertFalse(destination.exists())
            self.assertTrue((candidate / "Hermes Dev.app/Contents/Info.plist").is_file())
            destination.mkdir()
            rename(candidate, destination)

        with mock.patch.dict(namespace, rename_exclusive=competing_writer):
            with self.assertRaises(FileExistsError):
                self.bundle()
        self.assertTrue(self.output.is_dir())
        self.assertEqual(list(self.output.iterdir()), [])
        self.assertEqual(list(self.root.glob(".hermes-macos-dev-*")), [])

    def test_cli_rejects_bad_inputs_with_clear_failure(self):
        result = self.cli("--source-commit", "short")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("full 40-character", result.stderr)
        self.assert_no_artifact()

    def test_cli_requires_parent_brewfile(self):
        self.brewfile.unlink()
        result = self.cli()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Brewfile", result.stderr)
        self.assertIn("Bundle creation failed", result.stderr)
        self.assert_no_artifact()


if __name__ == "__main__":
    unittest.main()
