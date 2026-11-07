"""Check pinned release-check tools, or install them into an explicit private root."""

import argparse
import hashlib
import io
from pathlib import Path
import platform
import shutil
import subprocess
import tarfile
import tomllib
from urllib.request import urlopen


ROOT = Path(__file__).resolve().parents[1]
PINS = tomllib.loads((ROOT / "check-tools.toml").read_text())
TOOLS = (("cargo-deny", "cargo-deny"), ("typos-cli", "typos"), ("actionlint", "actionlint"))


def version(command):
    flag = "-version" if Path(command).name == "actionlint" else "--version"
    result = subprocess.run([command, flag], capture_output=True, text=True, check=True)
    words = result.stdout.split()
    if Path(command).name == "actionlint" and words:
        return words[0]
    if len(words) < 2:
        raise RuntimeError(f"Unrecognized version output from {command}")
    return words[1]


def install_actionlint(root, expected):
    machine = {"x86_64": "amd64", "aarch64": "arm64", "arm64": "arm64"}.get(platform.machine())
    system = platform.system().lower()
    if not machine or system not in ("linux", "darwin"):
        raise RuntimeError("Install the pinned actionlint manually on this platform")
    name = f"actionlint_{expected}_{system}_{machine}.tar.gz"
    url = f"https://github.com/rhysd/actionlint/releases/download/v{expected}/"
    with urlopen(url + f"actionlint_{expected}_checksums.txt", timeout=30) as response:
        checksums = dict(line.split()[::-1] for line in response.read(128 * 1024).decode().splitlines())
    with urlopen(url + name, timeout=30) as response:
        archive = response.read(64 * 1024 * 1024)
    if hashlib.sha256(archive).hexdigest() != checksums.get(name):
        raise RuntimeError("actionlint release checksum verification failed")
    destination = root.resolve() / "bin" / "actionlint"
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tarfile.open(fileobj=io.BytesIO(archive), mode="r:gz") as package:
        member = package.getmember("actionlint")
        if not member.isfile() or member.size > 32 * 1024 * 1024:
            raise RuntimeError("Unexpected actionlint archive member")
        with package.extractfile(member) as source:
            destination.write_bytes(source.read())
    destination.chmod(0o755)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--install", action="store_true", help="Explicitly install the pinned tools using Cargo")
    parser.add_argument("--root", type=Path, help="Private installation root; required with --install")
    parser.add_argument("--only", choices=[crate for crate, _ in TOOLS], help="Install/check one gate tool")
    args = parser.parse_args()
    if args.install and not args.root:
        parser.error("--install requires an explicit --root; global installations are not changed")
    if version("rustc") != PINS["rust"]:
        raise SystemExit(f"Use Rust {PINS['rust']} from rust-toolchain.toml")
    failures = []
    for crate, binary in TOOLS:
        if args.only and args.only != crate:
            continue
        command = str(args.root.resolve() / "bin" / binary) if args.root else shutil.which(binary)
        expected = PINS[crate]
        installed = version(command) if command and Path(command).is_file() else None
        if installed != expected and args.install:
            if crate == "actionlint":
                install_actionlint(args.root, expected)
            else:
                subprocess.run(["cargo", "install", "--locked", "--version", expected, "--root",
                                str(args.root.resolve()), "--jobs", "2", crate], cwd=ROOT, check=True)
            installed = version(command)
        if installed != expected:
            failures.append(f"{binary}: need {expected}; found {installed or 'not installed'}")
        else:
            print(f"Verified {binary} {installed}", flush=True)
    if failures:
        raise SystemExit("\n".join(failures) + "\nRun scripts/check-tools.py --install --root <private-tools-directory>, then add its bin directory to PATH.")


if __name__ == "__main__":
    main()
