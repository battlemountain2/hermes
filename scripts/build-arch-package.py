"""Build a checksum-bound native pacman test candidate; never installs it."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import tomllib


def run(*args, cwd=None, env=None):
    return subprocess.check_output(args, cwd=cwd, env=env, text=True).strip()


def prepare(source, sha, output):
    if not re.fullmatch(r"[a-f0-9]{40}", sha):
        raise ValueError("Use an approved full lowercase Git commit SHA")
    if run("git", "rev-parse", "HEAD", cwd=source) != sha:
        raise ValueError("Checkout must match the approved source commit")
    if run("git", "status", "--porcelain", cwd=source):
        raise ValueError("Refusing a dirty source checkout")
    manifest = tomllib.loads(run("git", "show", f"{sha}:Cargo.toml", cwd=source))
    version = manifest["package"]["version"]
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:-(?:rc|preview)\.[0-9]+)?", version):
        raise ValueError("Unsupported package version")
    output.mkdir(parents=True, exist_ok=False)
    archive = output / f"hermes-{sha}.tar"
    with archive.open("xb") as target:
        subprocess.run(["git", "archive", "--format=tar", f"--prefix=hermes-{sha}/", sha],
                       cwd=source, stdout=target, check=True)
    with archive.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
    template = run("git", "show", f"{sha}:packaging/arch/PKGBUILD.in", cwd=source)
    rendered = template.replace("@VERSION@", version.replace("-", "_"))
    rendered = rendered.replace("@SOURCE_COMMIT@", sha).replace("@SOURCE_SHA256@", digest)
    if "@SOURCE_" in rendered or "SKIP" in rendered:
        raise ValueError("Unresolved recipe identity")
    (output / "PKGBUILD").write_text(rendered + "\n")
    return version, digest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--sha", required=True)
    parser.add_argument("--output", type=Path, required=True, help="New output directory, outside the source checkout")
    parser.add_argument("--target-dir", type=Path, help="Optional isolated Cargo build cache")
    parser.add_argument("--prepare-only", action="store_true", help="Generate the source, checksum and recipe without compiling")
    args = parser.parse_args()
    source, output = args.source.resolve(), args.output.resolve()
    if output.is_relative_to(source):
        parser.error("Output must be outside the checkout")
    version, digest = prepare(source, args.sha, output)
    info = subprocess.check_output(["makepkg", "--printsrcinfo"], cwd=output, text=True)
    (output / ".SRCINFO").write_text(info)
    metadata = {"source_commit": args.sha, "version": version, "source_sha256": digest,
                "architecture": platform.machine(), "kind": "host-built test candidate",
                "clean_arch_install_verified": False, "asahi_verified": False, "macos_supported": False}
    (output / "BUILD-INFO.json").write_text(json.dumps(metadata, indent=2) + "\n")
    if args.prepare_only:
        print(output)
        return
    env = os.environ.copy()
    env["HERMES_BUILD_COMMIT"] = args.sha
    if args.target_dir:
        env["CARGO_TARGET_DIR"] = str(args.target_dir.resolve())
    # Start from the distribution config, but do not produce host-CPU-only code
    # or inherit user compiler/linker overrides into a candidate for other PCs.
    config = output / "makepkg.conf"
    config.write_text('source /etc/makepkg.conf\nCFLAGS="-O2 -pipe"\nCXXFLAGS="$CFLAGS"\n'
                      'RUSTFLAGS=""\nMAKEFLAGS="-j2"\nBUILDENV=(!distcc !color !ccache !check !sign)\n')
    for key in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CFLAGS", "CXXFLAGS", "LDFLAGS"):
        env.pop(key, None)
    subprocess.run(["makepkg", "--config", str(config), "--noconfirm"], cwd=output, env=env, check=True)
    packages = sorted(output.glob("*.pkg.tar.zst"))
    if not packages:
        raise RuntimeError("makepkg did not produce a package")
    lines = []
    for package in packages:
        with package.open("rb") as stream:
            lines.append(f"{hashlib.file_digest(stream, 'sha256').hexdigest()}  {package.name}\n")
        subprocess.run(["python3", str(source / "scripts/test_arch_package.py"), str(package), "--sha", args.sha], check=True)
    (output / "SHA256SUMS").write_text("".join(lines))
    print(f"Native test candidate built: {output}. Clean-machine and physical-device qualification remain pending.")


if __name__ == "__main__":
    main()
