import argparse
import os
import pathlib
import tempfile


APP_ID = "io.github.battlemountain2.Hermes"


def publish(destination, content, executable, replace):
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.is_symlink() or (destination.exists() and not destination.is_file()):
        raise ValueError(f"Refusing to replace a non-regular file: {destination}")
    if destination.exists() and not replace:
        if destination.read_bytes() == content:
            return
        raise FileExistsError(f"Already installed: {destination}; use --replace to update Hermes")
    descriptor, staging = tempfile.mkstemp(prefix=".hermes-install-", dir=destination.parent)
    try:
        with os.fdopen(descriptor, "wb") as output:
            output.write(content)
            output.flush()
            os.fchmod(output.fileno(), 0o755 if executable else 0o644)
        os.replace(staging, destination)
    finally:
        pathlib.Path(staging).unlink(missing_ok=True)


def desktop_exec(path):
    value = str(path)
    if any(ord(character) < 32 for character in value):
        raise ValueError("The installation path cannot contain control characters")
    if any(character in value for character in ("\\", '"', "`", "$")):
        raise ValueError("Choose an installation path without backslashes, quotes, dollar signs or backticks")
    return '"' + value.replace("%", "%%") + '" %U'


def install(root, binary, prefix, data_home, replace=False):
    binary = binary.resolve(strict=True)
    if not binary.is_file():
        raise ValueError("Choose a regular built Hermes binary")
    destination = prefix / "bin/hermes"
    execution = desktop_exec(destination)
    launcher = (root / "packaging/linux" / f"{APP_ID}.desktop").read_text()
    launcher = launcher.replace("Exec=hermes %U\n", f"Exec={execution}\n")
    launcher = launcher.replace("TryExec=hermes\n", f"TryExec={destination}\n")
    outputs = [(destination, binary.read_bytes(), True)]
    outputs.append((data_home / "applications" / f"{APP_ID}.desktop", launcher.encode(), False))
    development = (root / "packaging/linux" / f"{APP_ID}.Dev.desktop").read_text()
    development = development.replace("Exec=env HERMES_DEV=1 hermes %U\n", f"Exec=env HERMES_DEV=1 {execution}\n")
    development = development.replace("TryExec=hermes\n", f"TryExec={destination}\n")
    outputs.append((data_home / "applications" / f"{APP_ID}.Dev.desktop", development.encode(), False))
    for icon in sorted((root / "data/icons").glob(f"*/apps/{APP_ID}*")):
        relative = icon.relative_to(root / "data/icons")
        outputs.append((data_home / "icons/hicolor" / relative, icon.read_bytes(), False))
    for target, content, _ in outputs:
        if target.is_symlink() or (target.exists() and not target.is_file()):
            raise ValueError(f"Refusing to replace a non-regular file: {target}")
        if target.exists() and not replace and target.read_bytes() != content:
            raise FileExistsError(f"Already installed: {target}; use --replace to update Hermes")
    for target, content, executable in outputs:
        publish(target, content, executable, replace)
    return destination


if __name__ == "__main__":
    root = pathlib.Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description="Install Hermes's user-local binary, launcher and icon without changing defaults or starting it")
    parser.add_argument("--binary", type=pathlib.Path, default=root / "target/release/hermes")
    parser.add_argument("--prefix", type=pathlib.Path)
    parser.add_argument("--replace", action="store_true", help="Update existing regular Hermes installation files")
    args = parser.parse_args()
    prefix = (args.prefix or pathlib.Path.home() / ".local").expanduser().resolve()
    data_home = prefix / "share" if args.prefix else pathlib.Path(os.environ.get("XDG_DATA_HOME") or pathlib.Path.home() / ".local/share").expanduser().resolve()
    try:
        result = install(root, args.binary, prefix, data_home, args.replace)
    except (OSError, ValueError) as error:
        parser.exit(1, f"Hermes installation failed: {error}\n")
    print(f"Installed {result}\nNo application was started; file-manager and picker defaults are unchanged.")
