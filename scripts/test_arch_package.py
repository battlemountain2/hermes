"""Inspect a generated pacman artifact without installing or executing its app."""

import argparse
from pathlib import Path
import subprocess


def verify(package, sha):
    names = subprocess.check_output(["bsdtar", "-tf", str(package)], text=True).splitlines()
    expected = {"usr/bin/hermes", "usr/share/applications/io.github.battlemountain2.Hermes.desktop",
                "usr/share/applications/io.github.battlemountain2.Hermes.Dev.desktop",
                "usr/share/icons/hicolor/scalable/apps/io.github.battlemountain2.Hermes.svg",
                "usr/share/licenses/hermes-file-manager/LICENSE", "usr/share/doc/hermes/SOURCE_COMMIT"}
    missing = expected - set(names)
    if missing or any(n in names for n in (".INSTALL", "usr/bin/strata")):
        raise ValueError(f"Package layout failed; missing={sorted(missing)}; install hooks and Strata aliases are forbidden")
    for name in names:
        if name.startswith("/") or ".." in Path(name).parts:
            raise ValueError("Unsafe package member")
    read = lambda name: subprocess.check_output(["bsdtar", "-xOf", str(package), name], text=True)
    if read("usr/share/doc/hermes/SOURCE_COMMIT").strip() != sha:
        raise ValueError("Source identity mismatch")
    metadata = read(".PKGINFO")
    for dependency in ("bubblewrap", "glib2", "gstreamer", "util-linux"):
        if f"depend = {dependency}\n" not in metadata:
            raise ValueError(f"Missing runtime dependency: {dependency}")
    subprocess.run(["pacman", "-Qip", str(package)], check=True, stdout=subprocess.DEVNULL)
    print(f"Verified native package contents and metadata: {package.name}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("package", type=Path)
    parser.add_argument("--sha", required=True)
    args = parser.parse_args()
    verify(args.package.resolve(strict=True), args.sha)
