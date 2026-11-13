#!/usr/bin/env python3
"""Package Linux binary bundles and seal exact-source release assets; no native install."""

import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import tomllib

from release import RUST_VERSION, TARGETS, command, require, validate_source


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def write_json(path, value):
    with Path(path).open("x") as stream:
        json.dump(value, stream, indent=2, sort_keys=True)
        stream.write("\n")


def read_json(path):
    return json.loads(Path(path).read_text())


def build_identity(sha, version, channel, target):
    require(target in TARGETS, "unsupported Linux build target")
    return {"source_sha": sha, "version": version, "channel": channel, "target": target,
            "rust": RUST_VERSION, "workflow_sha": os.environ["GITHUB_SHA"],
            "run_id": os.environ["GITHUB_RUN_ID"]}


def dependency_inventory(metadata, lock, license_dir):
    require(metadata.get("version") == 1 and metadata.get("resolve", {}).get("nodes"), "resolved Cargo metadata is required")
    nodes = {node["id"] for node in metadata["resolve"]["nodes"]}
    checksums = {(p["name"], p["version"], p.get("source")): p.get("checksum") for p in lock["package"]}
    packages = []
    for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"], p["id"])):
        if package["id"] not in nodes:
            continue
        key = (package["name"], package["version"], package.get("source"))
        require(key in checksums, f"dependency absent from committed lockfile: {key}")
        root = Path(package["manifest_path"]).parent.resolve()
        candidates = {p for p in root.iterdir() if p.is_file() and p.name.upper().startswith(("LICENSE", "COPYING", "NOTICE", "COPYRIGHT"))}
        if package.get("license_file"):
            candidate = (root / package["license_file"]).resolve()
            require(candidate.is_relative_to(root) and candidate.is_file(), "dependency license file must be inside its crate")
            candidates.add(candidate)
        # Use an ID hash because multiple registries can provide the same name/version.
        folder = f"{package['name']}-{package['version']}-{hashlib.sha256(package['id'].encode()).hexdigest()[:12]}"
        texts = []
        for candidate in sorted(candidates):
            require(candidate.resolve().is_relative_to(root), "dependency license symlink escapes crate")
            destination = license_dir / folder / candidate.name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(candidate, destination)
            texts.append({"path": f"dependency-licenses/{folder}/{candidate.name}", "sha256": digest(destination)})
        packages.append({"name": package["name"], "version": package["version"], "source": package.get("source"),
                         "checksum": checksums[key], "license": package.get("license"),
                         "repository": package.get("repository"), "license_texts": texts})
    require(packages, "dependency inventory cannot be empty")
    return packages


def bundle(args):
    validate_source(args.source, args.sha, args.version)
    identity = build_identity(args.sha, args.version, args.channel, args.target)
    args.output.mkdir(parents=True, exist_ok=True)
    require(not list(args.output.iterdir()), "bundle output must be empty")
    binary = args.source / "target" / args.target / "release" / "hermes"
    require(binary.is_file(), "locked release binary is missing")
    rustc = command("rustc", f"+{RUST_VERSION}", "--version", "--verbose")
    require(f"release: {RUST_VERSION}\n" in rustc + "\n", "unexpected compiler version")
    prefix = f"hermes-{args.version}-{args.target}"
    with tempfile.TemporaryDirectory(prefix="hermes-bundle-") as temporary:
        root = Path(temporary) / prefix
        root.mkdir()
        shutil.copy2(binary, root / "hermes")
        for filename in ("README.md", "LICENSE", "THIRD_PARTY_LICENSES.md",
                         "packaging/linux/io.github.battlemountain2.Hermes.desktop",
                         "packaging/linux/io.github.battlemountain2.Hermes.Dev.desktop",
                         "data/icons/scalable/apps/io.github.battlemountain2.Hermes.svg"):
            shutil.copyfile(args.source / filename, root / Path(filename).name)
        for filename in ("scripts/desktop-integration.py", "docs/desktop-integration.md"):
            destination = root / filename
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(args.source / filename, destination)
        (root / "SOURCE_COMMIT").write_text(args.sha + "\n")
        lock = tomllib.loads((args.source / "Cargo.lock").read_text())
        metadata = read_json(args.metadata)
        packages = dependency_inventory(metadata, lock, root / "dependency-licenses")
        inventory = {**identity, "cargo_packages": packages,
                     "scope": "Resolved Cargo graph for this target and default features, including build dependencies. Declared licenses and available crate-root texts; not a legal compliance certification.",
                     "native_packages": command("dpkg-query", "-W", "-f=${binary:Package}\t${Version}\t${Architecture}\n").splitlines(),
                     "native_scope": "Build runner package inventory; not an exact runtime dependency closure or bundled native library license inventory.",
                     "elf_dynamic_section": command("readelf", "--dynamic", str(binary))}
        inventory_name = f"{prefix}.dependencies.json"
        write_json(args.output / inventory_name, inventory)
        shutil.copyfile(args.output / inventory_name, root / "DEPENDENCIES.json")
        build = {**identity, "rustc": rustc, "binary_sha256": digest(binary), "cargo_lock_sha256": digest(args.source / "Cargo.lock")}
        write_json(root / "BUILD.json", build)
        archive = args.output / f"{prefix}.tar.gz"
        with tarfile.open(archive, "x:gz") as tar:
            tar.add(root, arcname=prefix)
        build["assets"] = {archive.name: digest(archive), inventory_name: digest(args.output / inventory_name)}
        write_json(args.output / f"{prefix}.build.json", build)
    validate_source(args.source, args.sha, args.version)


def source_archive(source, sha, version, output):
    require(not output.exists(), "source archive already exists")
    with output.open("xb") as raw:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
            process = subprocess.Popen(["git", "archive", "--format=tar", f"--prefix=hermes-{version}/", sha], cwd=source, stdout=subprocess.PIPE)
            try:
                shutil.copyfileobj(process.stdout, compressed)
            finally:
                process.stdout.close()
                status = process.wait()
            require(status == 0, "git source archive failed")


def verify_files(directory, hashes):
    for name, checksum in hashes.items():
        require(Path(name).name == name and re_safe_name(name), "unsafe asset filename")
        path = directory / name
        require(not path.is_symlink() and path.is_file(), f"missing or unsafe asset: {name}")
        require(digest(path) == checksum, f"asset checksum mismatch: {name}")


def re_safe_name(name):
    import re
    return re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", name) is not None


def assemble(args):
    validate_source(args.source, args.sha, args.version)
    evidence = read_json(args.evidence)
    for key, value in (("source_sha", args.sha), ("version", args.version), ("channel", args.channel),
                       ("workflow_sha", os.environ["GITHUB_SHA"]), ("run_id", os.environ["GITHUB_RUN_ID"]), ("approval_verified", True)):
        require(evidence.get(key) == value, f"release evidence mismatch: {key}")
    builds = []
    expected = set()
    for target in TARGETS:
        prefix = f"hermes-{args.version}-{target}"
        name = f"{prefix}.build.json"
        build = read_json(args.output / name)
        for key, value in build_identity(args.sha, args.version, args.channel, target).items():
            require(build.get(key) == value, f"build identity mismatch for {target}: {key}")
        require(set(build["assets"]) == {f"{prefix}.tar.gz", f"{prefix}.dependencies.json"}, "unexpected build asset set")
        verify_files(args.output, build["assets"])
        inventory = read_json(args.output / f"{prefix}.dependencies.json")
        for key, value in build_identity(args.sha, args.version, args.channel, target).items():
            require(inventory.get(key) == value, f"inventory identity mismatch: {key}")
        require(inventory.get("cargo_packages") and inventory.get("native_packages"), "dependency inventory is empty")
        require(build.get("cargo_lock_sha256") == digest(args.source / "Cargo.lock"), "build used a different Cargo.lock")
        expected.update(build["assets"])
        expected.add(name)
        builds.append(build)
    require(set(p.name for p in args.output.iterdir()) == expected, "unexpected or missing downloaded release assets")
    archive = args.output / f"hermes-{args.version}-source.tar.gz"
    source_archive(args.source, args.sha, args.version, archive)
    hashes = {p.name: digest(p) for p in sorted(args.output.iterdir())}
    manifest = {"schema_version": 1, **evidence, "rust": RUST_VERSION, "builds": builds, "assets": hashes,
                "platform_scope": "Linux GNU binary bundles only. Neither macOS nor Asahi compatibility is certified. Native package qualification is separate.",
                "source_archive": {"filename": archive.name, "sha256": hashes[archive.name], "commit": args.sha}}
    write_json(args.output / "release-manifest.json", manifest)
    hashes["release-manifest.json"] = digest(args.output / "release-manifest.json")
    with (args.output / "SHA256SUMS").open("x") as stream:
        for name, checksum in sorted(hashes.items()):
            stream.write(f"{checksum}  {name}\n")
    verify_seal(args.output, args.sha, args.version, args.channel)


def verify_seal(directory, sha, version, channel):
    manifest = read_json(directory / "release-manifest.json")
    require(manifest.get("schema_version") == 1 and manifest.get("approval_verified") is True, "unapproved release manifest")
    for key, value in (("source_sha", sha), ("version", version), ("channel", channel), ("rust", RUST_VERSION),
                       ("workflow_sha", os.environ["GITHUB_SHA"]), ("run_id", os.environ["GITHUB_RUN_ID"])):
        require(manifest.get(key) == value, f"sealed release identity mismatch: {key}")
    hashes = dict(manifest["assets"])
    hashes["release-manifest.json"] = digest(directory / "release-manifest.json")
    verify_files(directory, hashes)
    expected = "".join(f"{checksum}  {name}\n" for name, checksum in sorted(hashes.items()))
    require((directory / "SHA256SUMS").read_text() == expected, "checksum file does not match sealed release")
    require(set(p.name for p in directory.iterdir()) == set(hashes) | {"SHA256SUMS"}, "asset set changed after sealing")
    require(not any(p.is_symlink() for p in directory.iterdir()), "symlink release asset")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("bundle", "assemble"))
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--sha", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--channel", choices=("preview", "rc", "stable"), required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--target", choices=TARGETS)
    parser.add_argument("--metadata", type=Path)
    parser.add_argument("--evidence", type=Path)
    args = parser.parse_args()
    if args.operation == "bundle":
        require(args.target and args.metadata, "bundle requires target and resolved metadata")
        bundle(args)
    else:
        require(args.evidence, "assemble requires verified release evidence")
        assemble(args)


if __name__ == "__main__":
    main()
