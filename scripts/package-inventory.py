"""Save the resolved Cargo licenses used by a native package build."""

import argparse
import json
from pathlib import Path
import subprocess
import tomllib

from release_artifacts import dependency_inventory


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    metadata = json.loads(subprocess.check_output(
        ["cargo", "+1.99.0", "metadata", "--locked", "--format-version", "1"], text=True))
    lock = tomllib.loads(Path("Cargo.lock").read_text())
    inventory = dependency_inventory(metadata, lock, args.output / "dependency-licenses")
    (args.output / "DEPENDENCIES.json").write_text(json.dumps({
        "scope": "Resolved Cargo default-feature graph, including build dependencies; native runtime dependencies are declared in .PKGINFO.",
        "packages": inventory}, indent=2) + "\n")
