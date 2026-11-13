"""Package a supplied native arm64 macos-dev executable without building or signing."""

import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import stat
import struct
import sys
import tempfile
import tomllib


SOURCE = Path(__file__).resolve().parents[1]
BUNDLE_ID = "io.github.battlemountain2.Hermes.Dev"
README = """Hermes Dev native test bundle

Requires an Apple Silicon Mac running macOS {deployment_target} or later.
This bundle depends on installed, dynamically linked Homebrew libraries.
In this output directory, install the packaged dependencies
before launching (Homebrew must already be installed for Apple Silicon).

    brew bundle --file ./Brewfile

The supplied executable must have been built with --features macos-dev.
That feature selects the Dev profile and application identity without shell
environment overrides. The bundler checks its Mach-O header, but cannot verify
the compiled feature selection or resolve its dynamic dependencies.

This packaging step performs no signing or notarization. Consult the native CI
signing and validation results before testing a downloaded candidate.

Start with an empty test folder. From this output directory, first collect
diagnostics and verify the Dev identity and profile
paths before opening the app. Record the actual OS and dependency versions.

    ./"Hermes Dev.app"/Contents/MacOS/hermes --diagnostics
    open "Hermes Dev.app"

This is a browse-first Dev candidate. Media and document decoding and playback
are unavailable until the isolated Mac preview service is implemented. Native
file-operation and macOS integration qualification are still pending.

Check that the first window opens, the identity is
io.github.battlemountain2.Hermes.Dev, and profile writes use only the Dev roots.
Record startup or missing-library failures and native CI results. Bundle
assembly and offline tests alone do not establish native runtime acceptance.
SOURCE-INFO.json records the declared source commit and packaged binary hash.
BUILD-INFO.json leaves uncollected build/dependency information null.
"""


def rename_exclusive(source, destination):
    """Publish atomically, including when another writer creates an empty output."""
    libc = ctypes.CDLL(None, use_errno=True)
    if sys.platform == "darwin":
        rename = libc.renamex_np
        rename.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_uint]
        arguments = (os.fsencode(source), os.fsencode(destination), 4)  # RENAME_EXCL
    elif sys.platform == "linux":
        rename = libc.renameat2
        rename.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_int,
                           ctypes.c_char_p, ctypes.c_uint]
        arguments = (-100, os.fsencode(source), -100, os.fsencode(destination), 1)
    else:
        raise ValueError("Atomic exclusive publication requires macOS or Linux")
    rename.restype = ctypes.c_int
    if rename(*arguments) != 0:
        error = ctypes.get_errno()
        raise OSError(error, os.strerror(error), str(destination))


def build_bundle(binary, output, source_commit, deployment_target="15.0", *, source=SOURCE):
    if not re.fullmatch(r"[0-9a-fA-F]{40}", source_commit):
        raise ValueError("source-commit must be a full 40-character hexadecimal Git SHA")
    if (not re.fullmatch(r"[0-9]+\.[0-9]+(?:\.[0-9]+)?", deployment_target)
            or int(deployment_target.split(".")[0]) < 15):
        raise ValueError("deployment-target must be a macOS version of 15.0 or later")
    binary, output = Path(binary).absolute(), Path(output).absolute()
    if binary.is_symlink():
        raise ValueError("binary path must not be a symlink")
    if os.path.lexists(output):
        raise FileExistsError(f"Refusing to overwrite output artifact {output}")
    with (source / "Cargo.toml").open("rb") as manifest:
        version = tomllib.load(manifest)["package"]["version"]
    if not isinstance(version, str) or not version.strip():
        raise ValueError("Cargo package version must be a nonempty string")
    icon = source / "packaging/macos/Hermes.icns"
    brewfile = source / "packaging/macos/Brewfile"
    # Open once, reject non-regular files, and preserve precisely these bytes.
    descriptor = os.open(binary, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, "rb") as executable:
        if not stat.S_ISREG(os.fstat(executable.fileno()).st_mode):
            raise ValueError("binary must be a regular file")
        header = executable.read(32)
        if len(header) != 32:
            raise ValueError("binary must contain a complete 64-bit Mach-O header")
        fields = struct.unpack("<8I", header)
        if fields[:2] != (0xFEEDFACF, 0x0100000C):
            raise ValueError("binary must be a thin little-endian arm64 Mach-O executable")
        if fields[3] != 2:  # MH_EXECUTE
            raise ValueError("Mach-O binary must have executable file type")
        output.parent.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix=".hermes-macos-dev-", dir=output.parent) as temporary:
            candidate = Path(temporary) / "candidate"
            contents = candidate / "Hermes Dev.app/Contents"
            (contents / "MacOS").mkdir(parents=True)
            (contents / "Resources").mkdir()
            packaged_binary = contents / "MacOS/hermes"
            executable.seek(0)
            with packaged_binary.open("xb") as target:
                shutil.copyfileobj(executable, target)
            packaged_binary.chmod(0o755)
            with packaged_binary.open("rb") as target:
                digest = hashlib.file_digest(target, "sha256").hexdigest()
            shutil.copyfile(icon, contents / "Resources/Hermes.icns")
            shutil.copyfile(brewfile, candidate / "Brewfile")
            plist = {
                "CFBundleExecutable": "hermes",
                "CFBundleIdentifier": BUNDLE_ID,
                "CFBundleName": "Hermes Dev",
                "CFBundleDisplayName": "Hermes Dev",
                "CFBundlePackageType": "APPL",
                "CFBundleVersion": version,
                "CFBundleShortVersionString": version,
                "LSMinimumSystemVersion": deployment_target,
                "NSHighResolutionCapable": True,
                "CFBundleIconFile": "Hermes.icns",
            }
            with (contents / "Info.plist").open("xb") as target:
                plistlib.dump(plist, target)
            provenance = {"source_commit": source_commit, "binary_sha256": digest,
                          "architecture": "arm64"}
            build_info = {"version": version, "bundle_id": BUNDLE_ID,
                          "deployment_target": deployment_target,
                          "dependency_versions": None, "sdk": None, "toolchain": None}
            for name, data in (("SOURCE-INFO.json", provenance), ("BUILD-INFO.json", build_info)):
                (candidate / name).write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
            (candidate / "README.txt").write_text(
                README.format(deployment_target=deployment_target), encoding="utf-8")
            rename_exclusive(candidate, output)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path,
                        help="Native arm64 executable built with --features macos-dev")
    parser.add_argument("--output", required=True, type=Path, help="New artifact directory")
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--deployment-target", default="15.0")
    args = parser.parse_args()
    try:
        build_bundle(args.binary, args.output, args.source_commit, args.deployment_target)
    except (OSError, ValueError, KeyError) as error:
        parser.exit(1, f"Bundle creation failed: {error}\n")
    print(f"Created {args.output / 'Hermes Dev.app'}")


if __name__ == "__main__":
    main()
