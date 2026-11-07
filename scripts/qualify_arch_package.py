"""Exercise a downloaded package inside an explicitly disposable Arch root only."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import tempfile
import time

from test_arch_package import verify


MARKER = "hermes-package-qualification-v1\n"
PACKAGE = "hermes-file-manager"


def require_disposable_root():
    marker = Path("/.hermes-disposable-root")
    if os.geteuid() != 0 or not marker.is_file() or marker.read_text() != MARKER:
        raise ValueError("Run only in a marked disposable Arch container/rootfs")
    if "ID=arch\n" not in Path("/etc/os-release").read_text():
        raise ValueError("Qualification requires an Arch userspace")
    mapping = Path("/proc/self/uid_map").read_text().split()
    private_root = len(mapping) >= 3 and mapping[0] == "0" and mapping[1] != "0"
    if not private_root and not Path("/.dockerenv").is_file():
        raise ValueError("A private user namespace or disposable Docker container is required")


def candidate_at(directory):
    info = json.loads((directory / "BUILD-INFO.json").read_text())
    sha = info["source_commit"]
    if not re.fullmatch(r"[0-9a-f]{40}", sha):
        raise ValueError("Artifact must record its full source commit")
    lines = (directory / "SHA256SUMS").read_text().splitlines()
    if len(lines) != 1:
        raise ValueError("Qualify exactly one package per artifact")
    match = re.fullmatch(r"([0-9a-f]{64})  (hermes-file-manager-[A-Za-z0-9_.+-]+\.pkg\.tar\.zst)", lines[0])
    if not match:
        raise ValueError("Invalid package checksum record")
    digest, name = match.groups()
    package = directory / name
    with package.open("rb") as stream:
        if hashlib.file_digest(stream, "sha256").hexdigest() != digest:
            raise ValueError("Downloaded package checksum mismatch")
    return package, sha, digest


def fixtures(root):
    """Known schema samples for package retention, not app-level import proof."""
    song = {"kind": "native", "value": list(b"/fixtures/song.flac")}
    collections = {"version": 1, "next_id": 2, "favorites": [song],
                   "playlists": [{"id": 1, "name": "Qualification", "tracks": [song]}],
                   "indexed": [], "played": []}
    queue = {"version": 1, "tracks": [], "order": [], "history": [], "current": None,
             "repeat": "Off", "shuffle": False, "position": 0, "volume": 0.4}
    content = {
        "config/hermes/settings.toml": 'mode = "noctalia"\ntheme = "azure-glow"\nsync_picker_theme = true\n',
        "config/hermes/trails.toml": 'schema_version = 1\ntrails = []\n',
        "config/hermes/folder_views.json": '{"version": 1, "views": {}}\n',
        "data/hermes/music-collections-v1.json": json.dumps(collections),
        "data/hermes/ratings-v1.json": json.dumps({"version": 1, "ratings": {"/fixtures/photo.jpg": 5}}),
        "state/hermes/music-queue-v1.json": json.dumps(queue),
        "config/hermes-dev/settings.toml": 'mode = "theme"\ntheme = "azure-glow"\n',
        "config/strata/settings.toml": 'mode = "theme"\ntheme = "legacy"\n',
        "data/strata/music-collections-v1.json": json.dumps(collections),
        "config/mimeapps.list": '[Default Applications]\ninode/directory=previous.desktop;\n',
        "config/xdg-desktop-portal/portals.conf": '[preferred]\ndefault=previous\n',
    }
    expected = {}
    for name, value in content.items():
        path = root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(value)
        expected[path] = path.read_bytes()
    return expected


def assert_retained(expected):
    for path, content in expected.items():
        if path.is_symlink() or not path.is_file() or path.read_bytes() != content:
            raise ValueError(f"Package transaction changed retained fixture {path}")


def stop(process):
    if process is not None and process.poll() is None:
        os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=5)


def smoke_startup(output):
    """Launch the installed binary on a private display with a fresh profile."""
    with tempfile.TemporaryDirectory(prefix="hermes-installed-startup-") as temporary:
        root = Path(temporary)
        env = os.environ.copy()
        for key in list(env):
            if key.startswith(("HERMES_", "STRATA_", "DBUS_")) or key in ("DISPLAY", "WAYLAND_DISPLAY"):
                env.pop(key)
        for key, name in (("XDG_CONFIG_HOME", "config"), ("XDG_DATA_HOME", "data"),
                          ("XDG_STATE_HOME", "state"), ("XDG_CACHE_HOME", "cache"),
                          ("XDG_RUNTIME_DIR", "runtime")):
            directory = root / name
            directory.mkdir(mode=0o700)
            env[key] = str(directory)
        folder = root / "Documents"
        folder.mkdir()
        (folder / "qualification.txt").write_text("Generated package startup fixture\n")
        env.update(GDK_BACKEND="broadway", BROADWAY_DISPLAY=f":{os.getpid()}",
                   GTK_A11Y="none", GTK_USE_PORTAL="0", GDK_DEBUG="no-portals",
                   G_DEBUG="fatal-criticals", GSETTINGS_BACKEND="memory", RUST_LOG="info",
                   GIO_USE_VFS="local", GIO_USE_VOLUME_MONITOR="unix")
        server = application = None
        log = output / "installed-startup.log"
        with (output / "private-display.log").open("x") as display_log, log.open("x") as app_log:
            try:
                server = subprocess.Popen(["gtk4-broadwayd", "--port", "0", "--address", "127.0.0.1",
                                           env["BROADWAY_DISPLAY"]], env=env, stdout=display_log,
                                          stderr=subprocess.STDOUT, start_new_session=True)
                time.sleep(0.2)
                if server.poll() is not None:
                    raise RuntimeError("Private package display failed to start")
                application = subprocess.Popen(["dbus-run-session", "--", "/usr/bin/hermes", str(folder)],
                                               env=env, stdout=app_log, stderr=subprocess.STDOUT,
                                               start_new_session=True)
                deadline = time.monotonic() + 20
                while time.monotonic() < deadline:
                    text = log.read_text()
                    if application.poll() is not None:
                        raise RuntimeError("Installed application exited during startup")
                    if "application window presented" in text and "first directory batch rendered" in text:
                        break
                    time.sleep(0.1)
                else:
                    raise RuntimeError("Installed application did not present and render its folder in time")
                time.sleep(0.5)
                if application.poll() is not None:
                    raise RuntimeError("Installed application exited after presenting its folder")
            finally:
                stop(application)
                stop(server)
        text = log.read_text()
        if "CRITICAL" in text or "Theme parser error" in text:
            raise RuntimeError("Installed application reported a GTK/style failure")


class Qualification:
    def __init__(self, output):
        self.output = output
        self.report = {
            "checks": {},
            "pending": ["previous-release upgrade", "interactive migration and recovery",
                        "optional-helper preview corpus", "physical Wayland/browser/audio",
                        "Asahi ARM64", "macOS"],
        }

    def save(self):
        (self.output / "RESULTS.json").write_text(json.dumps(self.report, indent=2) + "\n")

    def run(self, name, command, *, success=True, env=None):
        log = self.output / f"{name}.log"
        with log.open("x") as output:
            completed = subprocess.run(command, text=True, stdout=output,
                                       stderr=subprocess.STDOUT, env=env, timeout=900)
        result = subprocess.CompletedProcess(command, completed.returncode, log.read_text())
        if success and result.returncode != 0:
            raise RuntimeError(f"{name} failed with {result.returncode}; see its log")
        return result

    def passed(self, name):
        self.report["checks"][name] = "passed"
        self.save()

    def exercise(self, directory, expected_sha):
        package, sha, digest = candidate_at(directory)
        if sha != expected_sha:
            raise ValueError("Artifact source does not match the requested commit")
        verify(package, sha)
        self.report.update(source_commit=sha, package_sha256=digest,
                           architecture=os.uname().machine, scope="disposable Arch userspace")
        self.save()
        installed = self.run("initial-package-query", ["pacman", "-Q", PACKAGE], success=False)
        if installed.returncode == 0:
            raise ValueError("Start with a clean root without Hermes installed")
        with tempfile.TemporaryDirectory(prefix="hermes-package-profile-") as temporary:
            root = Path(temporary)
            expected = fixtures(root)
            offline = root / "offline-pacman.conf"
            offline.write_text("[options]\nArchitecture = auto\nSigLevel = Required DatabaseOptional\n"
                               "LocalFileSigLevel = Optional\n")
            failed = self.run("missing-dependencies", ["pacman", "--config", str(offline),
                              "-U", "--noconfirm", str(package)], success=False)
            if failed.returncode == 0 or "could not satisfy dependencies" not in failed.stdout:
                raise ValueError("Fresh root must demonstrate dependency refusal before installing libraries")
            if Path("/usr/bin/hermes").exists():
                raise ValueError("Failed transaction unexpectedly installed Hermes")
            assert_retained(expected)
            self.passed("offline missing-dependency refusal")
            metadata = subprocess.check_output(["bsdtar", "-xOf", str(package), ".PKGINFO"], text=True)
            dependencies = re.findall(r"^depend = (.+)$", metadata, re.MULTILINE)
            self.run("runtime-dependencies", ["pacman", "-Syu", "--needed", "--noconfirm", *dependencies])
            self.run("install", ["pacman", "-U", "--noconfirm", str(package)])
            assert_retained(expected)
            self.passed("clean installation and retained data")
            self.run("installed-file-integrity", ["pacman", "-Qkk", PACKAGE])
            files = self.run("installed-file-list", ["pacman", "-Qlq", PACKAGE]).stdout.splitlines()
            owned_files = [Path(name) for name in files if Path(name).is_file() or Path(name).is_symlink()]
            self.run("runtime-package-inventory", ["pacman", "-Q"])
            env = os.environ.copy()
            for key in ("HERMES_DEV", "STRATA_DEV", "DISPLAY", "WAYLAND_DISPLAY", "DBUS_SESSION_BUS_ADDRESS"):
                env.pop(key, None)
            for key, name in (("XDG_CONFIG_HOME", "config"), ("XDG_DATA_HOME", "data"),
                              ("XDG_STATE_HOME", "state"), ("XDG_CACHE_HOME", "cache")):
                env[key] = str(root / name)
            version = self.run("version", ["/usr/bin/hermes", "--version"], env=env).stdout
            if sha not in version or not version.startswith("Hermes "):
                raise ValueError("Installed executable does not match the source identity")
            diagnostics = self.run("diagnostics", ["/usr/bin/hermes", "--diagnostics"], env=env).stdout
            for label, directory in (("Config", "config"), ("Data", "data"),
                                     ("State", "state"), ("Cache", "cache")):
                if f"{label} Directory: {root}/{directory}/hermes\n" not in diagnostics:
                    raise ValueError(f"Stable {label} path is not isolated")
            dev_env = dict(env, HERMES_DEV="1")
            dev = self.run("development-diagnostics", ["/usr/bin/hermes", "--diagnostics"], env=dev_env).stdout
            if f"Config Directory: {root}/config/hermes-dev\n" not in dev:
                raise ValueError("Development paths are not isolated")
            capability = self.run("desktop-capabilities", ["/usr/bin/hermes", "--desktop-integration-info"], env=env)
            if not json.loads(capability.stdout).get("picker"):
                raise ValueError("Installed desktop integration capability is missing")
            assert_retained(expected)
            self.passed("installed executable CLI and development separation")
            smoke_startup(self.output)
            assert_retained(expected)
            self.passed("installed application private-display startup and folder rendering")
            self.run("reinstall", ["pacman", "-U", "--noconfirm", str(package)])
            assert_retained(expected)
            self.passed("same-version reinstall retention")
            self.run("uninstall", ["pacman", "-R", "--noconfirm", PACKAGE])
            if any(path.exists() or path.is_symlink() for path in owned_files):
                raise ValueError("Uninstall left package-owned files behind")
            assert_retained(expected)
            self.passed("uninstall retention and desktop defaults preservation")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--source-commit", required=True, help="Expected full package source SHA")
    parser.add_argument("--output", type=Path, required=True, help="New evidence directory")
    args = parser.parse_args()
    require_disposable_root()
    args.output.mkdir(parents=True, exist_ok=False)
    qualification = Qualification(args.output)
    try:
        qualification.exercise(args.candidate.resolve(strict=True), args.source_commit)
    except Exception as error:
        qualification.report["failure"] = str(error)
        raise
    finally:
        qualification.save()
    print(json.dumps(qualification.report, indent=2))


if __name__ == "__main__":
    main()
