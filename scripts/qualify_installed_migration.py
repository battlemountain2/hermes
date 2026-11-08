"""Exercise packaged Hermes import through private native accessibility controls."""

import argparse
from collections import deque
import hashlib
import json
import os
from pathlib import Path
import re
import select
import signal
import subprocess
import time
import tomllib
import wave

from qualify_arch_package import require_disposable_root, stop


IMPORT = "Import / resume Strata data"
CONTINUE = "Continue with Hermes data"
VERIFY_ERROR = "Import data changed or failed verification. Your files and backups were preserved."
RECOVERING = ("A previous Strata import did not finish. Resume from the local recovery record and backups, "
              "or continue with the Hermes data already present.")
REGISTRY = ("org.a11y.atspi.Registry", "/org/a11y/atspi/accessible/root")


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def wait_for(predicate, message, seconds=15):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(0.05)
    raise RuntimeError(message)


def journal(profile):
    directory = profile / "state/hermes/legacy-import-v2"
    generations = sorted(directory.glob("journal-*.json"))
    require(bool(generations), "Import did not write a recovery journal")
    return json.loads(generations[-1].read_text())


def seed(profile):
    documents = profile / "Documents"
    documents.mkdir()
    song = documents / "song.wav"
    with wave.open(str(song), "wb") as output:
        output.setnchannels(1)
        output.setsampwidth(2)
        output.setframerate(8000)
        output.writeframes(b"\0\0" * 800)
    (documents / "document.txt").write_text("Generated migration fixture\n")
    location = {"kind": "native", "value": list(os.fsencode(song))}
    collection = {"version": 1, "next_id": 2, "favorites": [location],
                  "playlists": [{"id": 1, "name": "Preserved playlist", "tracks": [location]}],
                  "indexed": [], "played": []}
    queue = {"version": 1, "tracks": [{"path": location, "tags": {"title": "Fixture song",
             "artist": "Fixture artist", "album": "Fixture album", "disc": 1, "track": 1}}],
             "order": [0], "history": [], "current": 0, "repeat": "Off",
             "shuffle": False, "position": 0, "volume": 0.4}
    folder = json.dumps(list(os.fsencode(documents)))
    content = {
        "config/settings.toml": b'mode = "noctalia"\ntheme = "graphite"\nstartup_location = "last"\n',
        "config/trails.toml": ('schema_version = 1\nactive = "saved"\n[[trails]]\n'
                              'id = "saved"\nname = "Saved folder"\ncustom_name = true\n'
                              f'locations = [{{kind = "native", value = {folder}}}]\n').encode(),
        "config/folder_views.json": b'{"version":1,"views":{}}',
        "data/music-collections-v1.json": json.dumps(collection).encode(),
        "data/ratings-v1.json": json.dumps({"version": 1, "ratings": {str(documents / "document.txt"): 4}}).encode(),
        "state/music-queue-v1.json": json.dumps(queue).encode(),
    }
    for name, value in content.items():
        source = source_path(profile, name)
        source.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        source.write_bytes(value)
    return content


def source_path(profile, name):
    root, relative = name.split("/", 1)
    return profile / root / "strata" / relative


def target_path(profile, name):
    root, relative = name.split("/", 1)
    return profile / root / "hermes" / relative


def check_originals(profile, content):
    for name, value in content.items():
        require(source_path(profile, name).read_bytes() == value, "Legacy original changed")


def check_imported(profile, content, preserved=()):
    for name, value in content.items():
        if name in preserved:
            continue
        actual = target_path(profile, name).read_bytes()
        if name.endswith(".json"):
            require(json.loads(actual) == json.loads(value), f"Imported JSON content differs for {name}")
        else:
            expected = tomllib.loads(value.decode())
            loaded = tomllib.loads(actual.decode())
            for key in expected:
                require(loaded.get(key) == expected[key], f"Imported TOML content differs for {name}")
    state = journal(profile)
    require(state["completed"] and not state["keep_hermes"], "Import did not finish")
    for index, entry in enumerate(state["entries"]):
        if entry["state"] == "Published":
            name = ["config", "data", "state"][entry["root"]] + "/" + entry["relative"]
            backup = profile / f"state/hermes/legacy-import-v2/backups/{index:03}.backup"
            require(backup.read_bytes() == content[name], "Original backup changed")
            require(backup.stat().st_mode & 0o777 == 0o600, "Backup permissions are not private")


def checkpoint(profile, content):
    directory = profile / "state/hermes/legacy-import-v2"
    backups = directory / "backups"
    backups.mkdir(mode=0o700, parents=True)
    directory.chmod(0o700)
    entries = []
    for index, (name, value) in enumerate(content.items()):
        root, relative = name.split("/", 1)
        entries.append({"root": ["config", "data", "state"].index(root), "relative": relative,
                        "bytes": len(value), "sha256": hashlib.sha256(value).hexdigest(), "state": "BackedUp"})
        backup = backups / f"{index:03}.backup"
        backup.write_bytes(value)
        backup.chmod(0o600)
    record = directory / "journal-0000.json"
    record.write_text(json.dumps({"version": 2, "completed": False, "keep_hermes": False, "entries": entries}))
    record.chmod(0o600)
    return record.read_bytes()


class Accessibility:
    def __init__(self, address):
        import gi
        gi.require_version("Gio", "2.0")
        from gi.repository import Gio, GLib
        self.Gio, self.GLib = Gio, GLib
        self.connection = Gio.DBusConnection.new_for_address_sync(
            address, Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION,
            None, None)

    def call(self, node, interface, method, parameters=None):
        return self.connection.call_sync(*node, interface, method, parameters, None,
                                         self.Gio.DBusCallFlags.NONE, 1000, None).unpack()

    def find(self, name, *, action=False):
        pending = deque([REGISTRY])
        visited = set()
        deadline = time.monotonic() + 3
        while pending and len(visited) < 512 and time.monotonic() < deadline:
            node = pending.popleft()
            if node in visited:
                continue
            visited.add(node)
            try:
                label = self.call(node, "org.freedesktop.DBus.Properties", "Get",
                                  self.GLib.Variant("(ss)", ("org.a11y.atspi.Accessible", "Name")))[0]
                if label == name:
                    interfaces = self.call(node, "org.a11y.atspi.Accessible", "GetInterfaces")[0]
                    if not action or "org.a11y.atspi.Action" in interfaces:
                        return node
                pending.extend(tuple(child) for child in self.call(node, "org.a11y.atspi.Accessible", "GetChildren")[0])
            except self.GLib.Error:
                # A setup window is replaced asynchronously after a choice or retry.
                if self.connection.is_closed():
                    raise
        return None

    def see(self, name):
        return wait_for(lambda: self.find(name), f"Accessible control or message missing: {name}")

    def click(self, name):
        node = wait_for(lambda: self.find(name, action=True), f"Accessible action missing: {name}")
        require(self.call(node, "org.a11y.atspi.Action", "DoAction", self.GLib.Variant("(i)", (0,)))[0],
                f"Accessible action failed: {name}")


class InstalledChecks:
    def __init__(self, output, address, display):
        self.output = output
        self.accessible = Accessibility(address)
        self.env = dict(os.environ, DISPLAY=display, GDK_BACKEND="x11", GTK_A11Y="atspi",
                        AT_SPI_BUS_ADDRESS=address, GTK_USE_PORTAL="0", GDK_DEBUG="no-portals",
                        G_DEBUG="fatal-criticals", GSETTINGS_BACKEND="memory", RUST_LOG="info", LC_ALL="C.UTF-8")
        self.process = None
        self.log = None
        self.report = {"scope": "installed binary with private X11/accessibility buses", "checks": {}, "pending": []}

    def profile(self, name):
        profile = self.output / name
        profile.mkdir(mode=0o700)
        for directory in ("config", "data", "state", "cache", "runtime"):
            (profile / directory).mkdir(mode=0o700)
        return profile

    def start(self, profile, label, *, development=False):
        self.close(force=True)
        env = self.env.copy()
        for key in list(env):
            if key.startswith(("HERMES_", "STRATA_")):
                env.pop(key)
        for key, name in (("XDG_CONFIG_HOME", "config"), ("XDG_DATA_HOME", "data"),
                          ("XDG_STATE_HOME", "state"), ("XDG_CACHE_HOME", "cache"), ("XDG_RUNTIME_DIR", "runtime")):
            env[key] = str(profile / name)
        if development:
            env["HERMES_DEV"] = "1"
        self.log = profile / f"{label}.log"
        command = ["/usr/bin/hermes", str(profile / "Documents")]
        with self.log.open("x") as log:
            self.process = subprocess.Popen(command, env=env, stdout=log, stderr=subprocess.STDOUT,
                                            start_new_session=True)

    def prompt(self):
        self.accessible.see("Choose your Hermes data")
        require("application window presented" not in self.log.read_text(), "Browser initialized before consent")

    def ready(self):
        wait_for(lambda: "first directory batch rendered" in self.log.read_text(), "Browser did not render the generated folder")
        require(self.process.poll() is None, "Installed browser exited unexpectedly")
        require(self.accessible.find("Choose your Hermes data") is None, "Setup prompt stayed open")

    def close(self, *, force=False):
        if self.process is None:
            return
        try:
            if not force:
                require(self.process.poll() is None, "Installed app exited before the normal window close")
                self.accessible.click("Close window")
                self.process.wait(timeout=10)
                require(self.process.returncode == 0, "Normal window close failed")
        finally:
            stop(self.process)
            self.process = None
        if self.log:
            text = self.log.read_text()
            require("CRITICAL" not in text and "Theme parser error" not in text, "GTK or stylesheet failure")

    def passed(self, name):
        self.report["checks"][name] = "passed"
        (self.output / "RESULTS.json").write_text(json.dumps(self.report, indent=2) + "\n")

    def consent(self):
        profile = self.profile("consent")
        content = seed(profile)
        name = "data/ratings-v1.json"
        existing = json.loads(content[name])
        existing["ratings"] = {str(profile / "Documents/document.txt"): 5}
        target = target_path(profile, name)
        target.parent.mkdir(parents=True)
        target.write_text(json.dumps(existing))
        self.start(profile, "first")
        self.prompt()
        require(not target_path(profile, "data/music-collections-v1.json").exists(), "Data copied before consent")
        require(not (profile / "state/hermes/legacy-import-v2").exists(), "Journal created before consent")
        self.accessible.click(IMPORT)
        self.ready()
        self.close()
        check_originals(profile, content)
        check_imported(profile, content, preserved=(name,))
        require(json.loads(target.read_text()) == existing, "Newer destination ratings were replaced")
        self.start(profile, "restart")
        self.ready()
        self.close()
        self.passed("consent, data preservation, backups and normal restart")

    def decline(self):
        profile = self.profile("decline")
        content = seed(profile)
        self.start(profile, "first")
        self.prompt()
        self.accessible.click(CONTINUE)
        self.ready()
        self.close()
        state = journal(profile)
        require(state["completed"] and state["keep_hermes"], "Continue choice was not recorded")
        require(not target_path(profile, "data/music-collections-v1.json").exists(), "Declined collection was imported")
        check_originals(profile, content)
        self.start(profile, "restart")
        self.ready()
        self.close()
        self.passed("decline persists across restart")

    def recover_backup(self, *, corrupt=False):
        profile = self.profile("corrupt-backup" if corrupt else "backup-recovery")
        content = seed(profile)
        selected = {"data/music-collections-v1.json": content["data/music-collections-v1.json"]}
        original_journal = checkpoint(profile, selected)
        source_path(profile, next(iter(selected))).unlink()
        backup = profile / "state/hermes/legacy-import-v2/backups/000.backup"
        if corrupt:
            backup.write_bytes(b"damaged backup")
        self.start(profile, "first")
        self.prompt()
        self.accessible.click(IMPORT)
        if corrupt:
            self.accessible.see(VERIFY_ERROR)
            require(not target_path(profile, next(iter(selected))).exists(), "Corrupt backup reached the destination")
            require(not journal(profile)["completed"], "Failed import was marked complete")
            backup.write_bytes(next(iter(selected.values())))
            self.accessible.click("Retry check")
            self.accessible.see(RECOVERING)
            self.accessible.click(IMPORT)
        self.ready()
        self.close()
        check_imported(profile, selected)
        require((profile / "state/hermes/legacy-import-v2/journal-0000.json").read_bytes() == original_journal,
                "Recovery overwrote an earlier journal generation")
        self.passed("corrupt backup fails visibly and retries safely" if corrupt else "durable backup recovers a missing original")

    def interrupted(self):
        profile = self.profile("interrupted")
        content = seed(profile)
        name = "data/music-collections-v1.json"
        content[name] += b" " * (2 * 1024 * 1024)
        source_path(profile, name).write_bytes(content[name])
        self.start(profile, "limited")
        self.prompt()
        subprocess.run(["prlimit", "--pid", str(self.process.pid), "--core=0",
                        "--fsize=1048576:1048576"], check=True)
        self.accessible.click(IMPORT)
        wait_for(lambda: self.process.poll() is not None, "Bounded import fixture did not interrupt")
        require(self.process.returncode == -signal.SIGXFSZ, "Import did not stop at the expected file-size limit")
        require(not journal(profile)["completed"], "Interrupted import was marked complete")
        require(not target_path(profile, name).exists(), "Incomplete collection was published")
        self.report["interruption_signal"] = "SIGXFSZ from a 1 MiB process file-size limit"
        self.close(force=True)
        self.start(profile, "resume")
        self.prompt()
        self.accessible.click(IMPORT)
        self.ready()
        self.close()
        check_originals(profile, content)
        check_imported(profile, content)
        stages = list((profile / "state/hermes/legacy-import-v2").rglob(".import-stage-*"))
        self.report["retained_crash_stages"] = len(stages)
        for stage in stages:
            require(not stage.is_symlink() and stage.is_dir() and stage.stat().st_mode & 0o777 == 0o700,
                    "Retained crash stage is not a private directory")
            require((stage / "content").stat().st_mode & 0o777 == 0o600, "Retained crash file is not private")
        if stages:
            self.report["pending"].append("private crash-stage retention and cleanup review")
        self.passed("abrupt process interruption resumes remaining data")

    def development(self):
        profile = self.profile("development")
        content = seed(profile)
        selected = {"data/music-collections-v1.json": content["data/music-collections-v1.json"]}
        original = checkpoint(profile, selected)
        self.start(profile, "development", development=True)
        self.ready()
        self.close()
        require((profile / "state/hermes/legacy-import-v2/journal-0000.json").read_bytes() == original,
                "Development changed Stable recovery data")
        require(not target_path(profile, "data/music-collections-v1.json").exists(), "Development imported Stable data")
        require(not journal(profile)["completed"], "Development completed Stable recovery")
        check_originals(profile, content)
        self.passed("development leaves legacy and Stable recovery data separate")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--source-commit", required=True)
    args = parser.parse_args()
    if not re.fullmatch(r"[0-9a-f]{40}", args.source_commit):
        parser.error("Use the full lowercase package source commit")
    require_disposable_root()
    require(bool(os.environ.get("DBUS_SESSION_BUS_ADDRESS")), "Use dbus-run-session in the disposable root")
    version = subprocess.check_output(["/usr/bin/hermes", "--version"], text=True)
    require(args.source_commit in version, "Installed app source identity differs")
    args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
    runtime = args.output / "runtime"
    runtime.mkdir(mode=0o700)
    address = f"unix:path={runtime}/bus"
    display = accessibility = suite = None
    with (args.output / "display.log").open("x") as display_log, (args.output / "accessibility.log").open("x") as a11y_log:
        try:
            display = subprocess.Popen(["Xvfb", "-displayfd", "1", "-screen", "0", "1280x900x24", "-nolisten", "tcp", "-noreset"],
                                       stdout=subprocess.PIPE, stderr=display_log, text=True, start_new_session=True)
            require(bool(select.select([display.stdout], [], [], 5)[0]), "Private X11 display did not start")
            number = display.stdout.readline().strip()
            require(number.isdigit(), "Private X11 display returned an invalid number")
            env = dict(os.environ, DISPLAY=f":{number}", AT_SPI_BUS_ADDRESS=address)
            accessibility = subprocess.Popen(["dbus-daemon", "--nofork", "--config-file=/usr/share/defaults/at-spi2/accessibility.conf",
                                              f"--address={address}"], env=env, stdout=a11y_log,
                                             stderr=subprocess.STDOUT, start_new_session=True)
            wait_for(lambda: (runtime / "bus").exists(), "Private accessibility bus did not start")
            suite = InstalledChecks(args.output, address, f":{number}")
            suite.report["source_commit"] = args.source_commit
            suite.consent()
            suite.decline()
            suite.recover_backup()
            suite.recover_backup(corrupt=True)
            suite.interrupted()
            suite.development()
        except Exception as error:
            if suite:
                suite.report["failure"] = str(error)
            raise
        finally:
            try:
                if suite:
                    try:
                        suite.close(force=True)
                    finally:
                        (args.output / "RESULTS.json").write_text(json.dumps(suite.report, indent=2) + "\n")
            finally:
                stop(accessibility)
                stop(display)


if __name__ == "__main__":
    main()
