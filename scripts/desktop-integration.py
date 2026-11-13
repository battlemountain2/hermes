import argparse
import base64
import configparser
import fcntl
import json
import os
import pathlib
import re
import shlex
import stat
import subprocess
import tempfile


APP_ID = "io.github.battlemountain2.Hermes"
BUS = "org.freedesktop.FileManager1"
CAPABILITY = b"HERMES_FILE_MANAGER1_V1"
LIMIT = 1024 * 1024


def absolute(value):
    path = pathlib.Path(value).expanduser()
    if not path.is_absolute() or ".." in path.parts:
        raise ValueError(f"Choose an absolute path without parent traversal: {path}")
    return path


def directories(path, create=False):
    current = pathlib.Path(path.anchor)
    for part in path.parts[1:]:
        current /= part
        try:
            metadata = current.lstat()
        except FileNotFoundError:
            if create:
                current.mkdir(mode=0o700)
            continue
        if not stat.S_ISDIR(metadata.st_mode):
            raise ValueError(f"Refusing a symlink or non-directory parent: {current}")


def read(path):
    directories(path.parent)
    try:
        descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    except FileNotFoundError:
        return None
    with os.fdopen(descriptor, "rb") as source:
        metadata = os.fstat(source.fileno())
        if (not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.getuid()
                or metadata.st_nlink != 1 or metadata.st_size > LIMIT
                or stat.S_IMODE(metadata.st_mode) & ~0o777):
            raise ValueError(f"Expected a bounded, owned, unshared regular file: {path}")
        content = source.read(LIMIT + 1)
        if len(content) > LIMIT:
            raise ValueError(f"Configuration exceeds the size limit: {path}")
        return content, stat.S_IMODE(metadata.st_mode)


def write(path, content, mode):
    directories(path.parent, create=True)
    read(path)
    descriptor, temporary = tempfile.mkstemp(prefix=".hermes-desktop-", dir=path.parent)
    try:
        with os.fdopen(descriptor, "wb") as destination:
            os.fchmod(destination.fileno(), mode)
            destination.write(content)
            destination.flush()
            os.fsync(destination.fileno())
        os.replace(temporary, path)
        directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        pathlib.Path(temporary).unlink(missing_ok=True)


def folder_default(content):
    lines = content.decode("utf-8").splitlines(keepends=True)
    section = None
    sections = 0
    keys = []
    for index, line in enumerate(lines):
        stripped = line.strip()
        if stripped.startswith("[") and stripped.endswith("]"):
            section = stripped[1:-1]
            if section == "Default Applications":
                sections += 1
        elif section == "Default Applications" and "=" in stripped:
            key, _ = stripped.split("=", 1)
            if key.strip() == "inode/directory":
                keys.append(index)
    if sections > 1 or len(keys) > 1:
        raise ValueError("Resolve duplicate folder-default entries before enabling Hermes")
    replacement = f"inode/directory={APP_ID}.desktop;\n"
    if keys:
        lines[keys[0]] = replacement
    elif sections:
        index = next(index for index, line in enumerate(lines)
                     if line.strip() == "[Default Applications]") + 1
        if not lines[index - 1].endswith("\n"):
            lines[index - 1] += "\n"
        lines.insert(index, replacement)
    else:
        if lines and not lines[-1].endswith("\n"):
            lines[-1] += "\n"
        lines.extend(["\n[Default Applications]\n", replacement])
    return "".join(lines).encode("utf-8")


def encode(content):
    return None if content is None else base64.b64encode(content).decode("ascii")


def decode(content):
    return None if content is None else base64.b64decode(content, validate=True)


def refresh_activation():
    if not os.environ.get("DBUS_SESSION_BUS_ADDRESS"):
        return False
    try:
        result = subprocess.run([
            "gdbus", "call", "--session", "--dest", "org.freedesktop.DBus",
            "--object-path", "/org/freedesktop/DBus", "--method", "org.freedesktop.DBus.ReloadConfig",
        ], capture_output=True, timeout=5, check=False)
    except (OSError, subprocess.TimeoutExpired):
        return False
    return result.returncode == 0


def compatible_binary(binary, capability=CAPABILITY, label="FileManager1"):
    binary = absolute(binary)
    directories(binary.parent)
    descriptor = os.open(binary, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, "rb") as source:
        metadata = os.fstat(source.fileno())
        if not stat.S_ISREG(metadata.st_mode) or not os.access(binary, os.X_OK):
            raise ValueError("Install an executable Hermes build first")
        if source.read(4) != b"\x7fELF" or metadata.st_size > 256 * LIMIT:
            raise ValueError("Expected a bounded Linux Hermes executable")
        overlap = b""
        while chunk := source.read(65536):
            window = overlap + chunk
            if capability in window:
                return binary
            overlap = window[-len(capability):]
    raise ValueError(f"Install the new {label}-capable Hermes build before enabling")


class Setup:
    def __init__(self, config, data, state, desktop=""):
        self.config, self.data, self.state = map(absolute, (config, data, state))
        desktop = desktop.split(":", 1)[0].lower()
        if desktop and not re.fullmatch(r"[a-z0-9_-]{1,64}", desktop):
            raise ValueError("Unrecognized desktop name; no configuration was changed")
        self.mime = self.config / (f"{desktop}-mimeapps.list" if desktop else "mimeapps.list")
        self.preference = self.config / "hermes/folder-integration.json"
        self.service = self.data / "dbus-1/services/org.freedesktop.FileManager1.service"
        self.journal = self.state / "hermes/folder-integration-transaction.json"

    def plan(self, binary):
        binary = compatible_binary(binary)
        launcher = read(self.data / "applications" / f"{APP_ID}.desktop")
        if launcher is None:
            raise ValueError("Install the Hermes launcher first")
        entry = configparser.ConfigParser(interpolation=None)
        entry.read_string(launcher[0].decode("utf-8"))
        values = entry["Desktop Entry"]
        execution = shlex.split(values.get("Exec", ""))
        if (execution != [str(binary).replace("%", "%%"), "%U"]
                or "inode/directory" not in values.get("MimeType", "").split(";")):
            raise ValueError("The installed Hermes launcher must open folders with the chosen binary")
        self.check_service(BUS, "reveal")
        before = read(self.mime)
        content = folder_default(before[0] if before else b"")
        service = f"[D-BUS Service]\nName={BUS}\nExec={shlex.quote(str(binary))} --gapplication-service\n"
        if any(ord(character) < 32 for character in str(binary)):
            raise ValueError("Executable paths cannot contain control characters")
        outputs = [
            (self.mime, content, before[1] if before else 0o600),
            (self.service, service.encode("utf-8"), 0o644),
            (self.preference, b'{"version":1,"enabled":true}\n', 0o600),
        ]
        return self.snapshot(outputs)

    def check_service(self, bus, label):
        services = self.service.parent
        directories(services)
        if services.exists():
            entries = list(services.iterdir())
            if len(entries) > 512:
                raise ValueError("Too many user D-Bus services to check safely")
            for path in entries:
                if path.suffix != ".service" or path == self.service:
                    continue
                existing = read(path)
                if existing and re.search(rb"(?m)^Name\s*=\s*" + re.escape(bus.encode()) + rb"\s*$", existing[0]):
                    raise ValueError(f"Another user {label} service exists: {path}; review it first")

    def snapshot(self, outputs):
        changes = []
        for path, after, mode in outputs:
            previous = read(path)
            changes.append(dict(path=str(path), before=encode(previous[0]) if previous else None,
                                before_mode=previous[1] if previous else None,
                                after=encode(after), after_mode=mode))
        journal = dict(version=1, phase="prepared", changes=changes)
        if len(json.dumps(journal).encode("utf-8")) > LIMIT - 1024:
            raise ValueError("The recovery snapshot would exceed its size limit")
        return journal

    def validate_paths(self, paths):
        if (len(paths) != 3 or paths[1:] != [self.service, self.preference] or paths[0].parent != self.config
                or not re.fullmatch(r"(?:[a-z0-9_-]{1,64}-)?mimeapps\.list", paths[0].name)):
            raise ValueError("Integration journal names unexpected configuration files")

    def ordered_changes(self, journal):
        return journal["changes"]

    def load(self):
        record = read(self.journal)
        if record is None:
            return None
        journal = json.loads(record[0])
        if journal.get("version") != 1 or journal.get("phase") not in ("prepared", "enabled"):
            raise ValueError("Unrecognized integration recovery journal")
        changes = journal.get("changes", [])
        paths = [absolute(change["path"]) for change in changes]
        self.validate_paths(paths)
        for change in changes:
            if (change["after_mode"] not in (0o600, 0o644)
                    and change["path"] != str(paths[0])):
                raise ValueError("Invalid recovery permissions")
            for key in ("before", "after"):
                value = decode(change[key])
                if value is not None and len(value) > LIMIT:
                    raise ValueError("Recovery file exceeds its size limit")
            if change["after"] is None or (change["before"] is None) != (change["before_mode"] is None):
                raise ValueError("Incomplete recovery file")
            for key in ("before_mode", "after_mode"):
                if change[key] is not None and (not isinstance(change[key], int) or change[key] & ~0o777):
                    raise ValueError("Invalid recovery permissions")
        return journal

    def verify(self, journal, restoring=False):
        for change in journal["changes"]:
            current = read(absolute(change["path"]))
            actual = current if current else (None, None)
            allowed = [(decode(change["after"]), change["after_mode"])]
            if restoring:
                allowed.append((decode(change["before"]), change["before_mode"]))
            if actual not in allowed:
                raise ValueError(f"Configuration changed outside Hermes; refusing to overwrite: {change['path']}")

    def save(self, journal):
        content = json.dumps(journal).encode("utf-8") + b"\n"
        if len(content) > LIMIT:
            raise ValueError("The recovery snapshot exceeds its size limit")
        write(self.journal, content, 0o600)

    def enable(self, binary):
        previous = self.load()
        if previous:
            if previous["phase"] != "enabled":
                raise ValueError("An interrupted setup needs restore before enabling again")
            self.verify(previous)
            return
        journal = self.plan(binary)
        self.save(journal)
        for change in self.ordered_changes(journal):
            path = absolute(change["path"])
            current = read(path)
            if (current if current else (None, None)) != (decode(change["before"]), change["before_mode"]):
                raise ValueError(f"Configuration changed during setup: {path}; run restore")
            write(path, decode(change["after"]), change["after_mode"])
        journal["phase"] = "enabled"
        self.save(journal)

    def restore(self):
        journal = self.load()
        if journal is None:
            return
        self.verify(journal, restoring=True)
        for change in reversed(self.ordered_changes(journal)):
            path = absolute(change["path"])
            content = decode(change["before"])
            current = read(path)
            actual = current if current else (None, None)
            if actual == (content, change["before_mode"]):
                continue
            if actual != (decode(change["after"]), change["after_mode"]):
                raise ValueError(f"Configuration changed during restoration: {path}")
            if content is None:
                if current is not None:
                    path.unlink()
            else:
                write(path, content, change["before_mode"])
        self.journal.unlink()

    def lock(self):
        path = self.journal.with_suffix(".lock")
        directories(path.parent, create=True)
        descriptor = os.open(path, os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW | os.O_NONBLOCK, 0o600)
        file = os.fdopen(descriptor, "r+")
        metadata = os.fstat(file.fileno())
        if (not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.getuid()
                or metadata.st_nlink != 1):
            file.close()
            raise ValueError("Unsafe integration lock file")
        try:
            fcntl.flock(file.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError:
            file.close()
            raise ValueError("Another Hermes integration setup is running") from None
        return file


PICKER_BUS = "org.freedesktop.impl.portal.desktop.hermes"
PICKER_INTERFACE = "org.freedesktop.impl.portal.FileChooser"
PICKER_MANIFEST = ("[portal]\nDBusName=" + PICKER_BUS + "\nInterfaces=" + PICKER_INTERFACE + ";\nUseIn=\n").encode()


def portal_override(content):
    parser = configparser.ConfigParser(interpolation=None, strict=True)
    parser.optionxform = str
    parser.read_string(content.decode("utf-8"))
    if "preferred" not in parser:
        raise ValueError("The existing portal configuration has no preferred providers")
    lines = content.decode("utf-8").splitlines(keepends=True)
    section = None
    inserted = False
    output = []
    for line in lines:
        stripped = line.strip()
        if stripped.startswith("[") and stripped.endswith("]"):
            section = stripped[1:-1]
            output.append(line if line.endswith("\n") else line + "\n")
            if section == "preferred":
                output.append(f"{PICKER_INTERFACE}=hermes\n")
                inserted = True
            continue
        if section == "preferred" and "=" in line and line.split("=", 1)[0].strip() == PICKER_INTERFACE:
            continue
        output.append(line)
    if not inserted:
        raise ValueError("Unable to locate the preferred-provider section")
    return "".join(output).encode("utf-8")


class PickerSetup(Setup):
    def __init__(self, config, data, state, desktop="", config_dirs=None, data_dirs=None):
        super().__init__(config, data, state, desktop)
        self.desktops = [value.lower() for value in desktop.split(":") if value]
        if any(not re.fullmatch(r"[a-z0-9_-]{1,64}", value) for value in self.desktops):
            raise ValueError("Unrecognized desktop name")
        self.portal_config = self.config / "xdg-desktop-portal" / (f"{self.desktops[0]}-portals.conf" if self.desktops else "portals.conf")
        self.service = self.data / f"dbus-1/services/{PICKER_BUS}.service"
        self.manifest = self.data / "xdg-desktop-portal/portals/hermes.portal"
        self.preference = self.config / "hermes/picker-integration.json"
        self.journal = self.state / "hermes/picker-integration-transaction.json"
        config_dirs = config_dirs if config_dirs is not None else (os.environ.get("XDG_CONFIG_DIRS") or "/etc/xdg").split(":")
        data_dirs = data_dirs if data_dirs is not None else (os.environ.get("XDG_DATA_DIRS") or "/usr/local/share:/usr/share").split(":")
        self.roots = list(dict.fromkeys([self.config, *map(absolute, config_dirs), pathlib.Path("/etc"), self.data, *map(absolute, data_dirs), pathlib.Path("/usr/share")]))

    def baseline(self):
        if os.environ.get("XDG_DESKTOP_PORTAL_DIR"):
            raise ValueError("A custom XDG_DESKTOP_PORTAL_DIR overrides discovery; review it before configuring Hermes")
        names = [f"{desktop}-portals.conf" for desktop in self.desktops] + ["portals.conf"]
        for root in self.roots:
            for name in names:
                path = root / "xdg-desktop-portal" / name
                directories(path.parent)
                try:
                    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
                except FileNotFoundError:
                    continue
                with os.fdopen(descriptor, "rb") as source:
                    metadata = os.fstat(source.fileno())
                    if not stat.S_ISREG(metadata.st_mode) or metadata.st_size > LIMIT or metadata.st_uid not in (0, os.getuid()):
                        raise ValueError(f"Unsafe inherited portal configuration: {path}")
                    content = source.read(LIMIT + 1)
                    if len(content) > LIMIT:
                        raise ValueError("Inherited portal configuration is too large")
                return content
        raise ValueError("No existing portal configuration was found; refusing to guess providers for other interfaces")

    def plan(self, binary):
        binary = compatible_binary(binary, b"HERMES_FILE_CHOOSER_V1", "Open/Save-picker")
        self.check_service(PICKER_BUS, "picker")
        if any(ord(character) < 32 for character in str(binary)):
            raise ValueError("Executable paths cannot contain control characters")
        baseline = self.baseline()
        previous = read(self.portal_config)
        environment = pathlib.Path("/usr/bin/env")
        if not environment.is_file() or not os.access(environment, os.X_OK):
            raise ValueError("The picker backend requires /usr/bin/env for isolated GTK startup")
        service = f"[D-BUS Service]\nName={PICKER_BUS}\nExec={environment} GDK_DEBUG=no-portals GTK_USE_PORTAL=0 {shlex.quote(str(binary))} --portal-file-chooser\n"
        return self.snapshot([
            (self.portal_config, portal_override(baseline), previous[1] if previous else 0o600),
            (self.service, service.encode(), 0o644),
            (self.manifest, PICKER_MANIFEST, 0o644),
            (self.preference, b'{"version":1,"enabled":true}\n', 0o600),
        ])

    def validate_paths(self, paths):
        if (len(paths) != 4 or paths[1:] != [self.service, self.manifest, self.preference]
                or paths[0].parent != self.config / "xdg-desktop-portal"
                or not re.fullmatch(r"(?:[a-z0-9_-]{1,64}-)?portals\.conf", paths[0].name)):
            raise ValueError("Integration journal names unexpected configuration files")

    def ordered_changes(self, journal):
        changes = journal["changes"]
        return [changes[1], changes[3], changes[2], changes[0]]


def main():
    parser = argparse.ArgumentParser(description="Preview or explicitly enable/restore independent Hermes folder or Open/Save integration. Never starts an application or restarts portals.")
    parser.add_argument("action", nargs="?", default="status", choices=("status", "enable", "restore"))
    parser.add_argument("--apply", action="store_true", help="Write the explicitly requested setup or restoration")
    parser.add_argument("--component", choices=("folders", "picker"), default="folders", help="Independent integration to configure; defaults to folders")
    parser.add_argument("--binary", type=pathlib.Path, default=pathlib.Path.home() / ".local/bin/hermes")
    args = parser.parse_args()
    home = pathlib.Path.home()
    try:
        kind = PickerSetup if args.component == "picker" else Setup
        setup = kind(os.environ.get("XDG_CONFIG_HOME") or home / ".config",
                      os.environ.get("XDG_DATA_HOME") or home / ".local/share",
                      os.environ.get("XDG_STATE_HOME") or home / ".local/state",
                      os.environ.get("XDG_CURRENT_DESKTOP", ""))
        if args.action == "status":
            journal = setup.load()
            if journal:
                setup.verify(journal, restoring=journal["phase"] == "prepared")
            print(f"{args.component.title()} configuration: {journal['phase'] if journal else 'not managed by Hermes'}; live owner not checked")
        elif not args.apply:
            journal = setup.plan(args.binary) if args.action == "enable" else setup.load()
            if journal:
                for change in journal["changes"]:
                    print(f"Would {args.action}: {change['path']}")
            print("Preview only. Add --apply to make this change.")
        else:
            with setup.lock():
                if args.action == "enable":
                    setup.enable(args.binary)
                else:
                    setup.restore()
            if not refresh_activation():
                print("D-Bus activation metadata could not be refreshed; log out/in yourself before testing activation.")
            print(f"{args.component.title()} setup enabled. Existing service owners are never replaced."
                  if args.action == "enable" else f"Previous {args.component} settings restored; no application was restarted.")
            if args.component == "picker":
                print("Log out/in yourself to reload portal discovery. No portal service was restarted; avoid restarting portals during screen sharing or open dialogs.")
            else:
                print("Reopen Hermes yourself if it is already running.")
        print("Open/Save picker unchanged. No application, desktop service or audio was started." if args.component == "folders"
              else "Folder defaults unchanged. No application, portal service or audio was started or restarted.")
    except (OSError, ValueError, KeyError, TypeError, configparser.Error) as error:
        parser.exit(1, f"Hermes desktop setup stopped: {error}\nExisting recovery records are kept; review or run restore before retrying.\n")


if __name__ == "__main__":
    main()
