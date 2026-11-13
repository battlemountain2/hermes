import argparse
import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
from xml.sax.saxutils import escape


sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[1]
APP = "io.github.battlemountain2.Hermes"
BUS = "org.freedesktop.FileManager1"


def until(condition):
    deadline = time.monotonic() + 8
    while not condition():
        if time.monotonic() > deadline:
            raise AssertionError("Timed out waiting for private fixture state")
        time.sleep(0.03)


def check_private(root, binary):
    for variable, directory in [("HOME", "home"), ("XDG_CONFIG_HOME", "config"),
                                ("XDG_DATA_HOME", "data"), ("XDG_STATE_HOME", "state")]:
        if os.environ.get(variable) != str(root / directory):
            raise RuntimeError("This check requires the generated private environment")
    if os.environ.get("HERMES_DESKTOP_TEST_ROOT") != str(root) or os.environ.get("GDK_BACKEND") != "broadway":
        raise RuntimeError("Use the parent command to create a private display and session")
    capability = json.loads(subprocess.check_output([str(binary), "--desktop-integration-info"], text=True, timeout=5))
    assert capability["capability"] == "HERMES_FILE_MANAGER1_V1" and capability["folders"]
    spec = importlib.util.spec_from_file_location("desktop_setup", ROOT / "scripts/desktop-integration.py")
    setup = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(setup)
    integration = setup.Setup(root / "config", root / "data", root / "state", "HermesTest")
    applications = root / "data/applications"
    applications.mkdir()
    (applications / f"{APP}.desktop").write_text(
        f'[Desktop Entry]\nType=Application\nName=Hermes\nExec="{str(binary).replace("%", "%%")}" %U\nMimeType=inode/directory;\n')
    (applications / "generated-previous.desktop").write_text(
        "[Desktop Entry]\nType=Application\nName=Previous fixture handler\nExec=/bin/false %U\nMimeType=inode/directory;\n")
    (root / "config/mimeapps.list").write_text(
        "[Default Applications]\ninode/directory=generated-previous.desktop;\n")
    mime = lambda: subprocess.check_output(["gio", "mime", "inode/directory"], text=True, timeout=5).splitlines()[0]
    baseline = mime()
    assert "generated-previous.desktop" in baseline
    with integration.lock():
        integration.enable(binary)
    assert setup.refresh_activation()
    assert APP in mime()

    def dbus(method, *arguments, destination="org.freedesktop.DBus", path="/org/freedesktop/DBus", check=True):
        return subprocess.run([
            "gdbus", "call", "--session", "--dest", destination, "--object-path", path,
            "--method", method, *arguments,
        ], capture_output=True, text=True, check=check, timeout=8)

    process_handle = None
    try:
        dbus("org.freedesktop.DBus.StartServiceByName", BUS, "0")
        process = dbus("org.freedesktop.DBus.GetConnectionUnixProcessID", BUS).stdout
        process_id = int(process.split("uint32 ")[1].split(",")[0].split(")")[0])
        process_handle = os.pidfd_open(process_id)
        log = lambda: (root / "runtime.log").read_text(errors="replace")
        assert "application window presented" not in log(), "Service mode opened a window before a request"
        folder = root / "home/generated"
        folder.mkdir()
        first, second = folder / "first.txt", folder / "second.txt"
        first.write_text("Generated first file")
        second.write_text("Generated second file")
        dbus(BUS + ".ShowItems", json.dumps([first.as_uri(), second.as_uri()]), "''",
             destination=BUS, path="/org/freedesktop/FileManager1")
        until(lambda: "directory load finished" in log())
        assert log().count("application window presented") == 1
        invalid = dbus(BUS + ".ShowItems", '["https://example.com/not-a-file"]', "''",
                       destination=BUS, path="/org/freedesktop/FileManager1", check=False)
        assert invalid.returncode != 0 and "InvalidArgs" in invalid.stderr
        dbus(BUS + ".ShowItemProperties", json.dumps([first.as_uri()]), "''",
             destination=BUS, path="/org/freedesktop/FileManager1")
        until(lambda: log().count("application window presented") == 2)
        with integration.lock():
            integration.restore()
        assert setup.refresh_activation()
        until(lambda: "false" in dbus("org.freedesktop.DBus.NameHasOwner", BUS).stdout)
        os.kill(process_id, 0)
        assert mime() == baseline
        assert not integration.preference.exists() and not integration.service.exists()
        print("PASS: private activation, no startup window, grouped reveal, properties routing, invalid URI rejection, live opt-out and previous-default restoration", flush=True)
    finally:
        if process_handle is not None:
            try:
                signal.pidfd_send_signal(process_handle, signal.SIGTERM)
            except ProcessLookupError:
                pass
            os.close(process_handle)


def main():
    parser = argparse.ArgumentParser(description="Exercise a built Hermes on a private D-Bus, Broadway display and generated XDG roots. Never installs or changes real desktop defaults.")
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--private-root", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    if args.private_root:
        check_private(args.private_root, binary)
        return
    root = Path(tempfile.mkdtemp(prefix="hermes-desktop-runtime-"))
    environment = os.environ.copy()
    for key in ["DBUS_SESSION_BUS_ADDRESS", "DBUS_STARTER_ADDRESS", "DBUS_STARTER_BUS_TYPE",
                "WAYLAND_DISPLAY", "DISPLAY", "HERMES_DEV", "STRATA_DEV"]:
        environment.pop(key, None)
    for key, directory in [("HOME", "home"), ("XDG_CONFIG_HOME", "config"), ("XDG_DATA_HOME", "data"),
                           ("XDG_STATE_HOME", "state"), ("XDG_CACHE_HOME", "cache"), ("XDG_RUNTIME_DIR", "runtime")]:
        path = root / directory
        path.mkdir(mode=0o700)
        environment[key] = str(path)
    environment.update(
        HERMES_DESKTOP_TEST_ROOT=str(root), GDK_BACKEND="broadway", BROADWAY_DISPLAY=":3413",
        GTK_A11Y="none", GTK_USE_PORTAL="0", GIO_USE_VFS="local", GIO_USE_VOLUME_MONITOR="unix",
        GSETTINGS_BACKEND="memory", XDG_CURRENT_DESKTOP="HermesTest", XDG_CONFIG_DIRS="/etc/xdg",
        XDG_DATA_DIRS="/usr/local/share:/usr/share", RUST_LOG="hermes=debug",
    )
    configuration = root / "bus.conf"
    configuration.write_text(
        '<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN" "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">\n'
        '<busconfig><type>session</type><listen>unix:tmpdir=/tmp</listen><auth>EXTERNAL</auth>'
        f'<servicedir>{escape(str(root / "data/dbus-1/services"))}</servicedir>'
        f'<policy user="{os.getuid()}"><allow own="*"/><allow send_destination="*"/><allow receive_sender="*"/></policy></busconfig>')
    print(f"Private validation logs: {root}", flush=True)
    with (root / "broadway.log").open("w") as display_log, (root / "runtime.log").open("w") as log:
        display = subprocess.Popen(["gtk4-broadwayd", "-a", "127.0.0.1", "-p", "0", ":3413"],
                                   env=environment, stdout=display_log, stderr=display_log)
        try:
            time.sleep(0.2)
            assert display.poll() is None, "Unable to start private Broadway display"
            result = subprocess.run([
                "dbus-run-session", "--config-file", str(configuration), "--", sys.executable,
                __file__, "--private-root", str(root), "--binary", str(binary),
            ], env=environment, stdout=log, stderr=log, timeout=45)
            if result.returncode:
                raise RuntimeError(f"Private desktop check failed; see {root / 'runtime.log'}")
            print("Private desktop integration check passed; real defaults and windows are unchanged.")
        finally:
            display.terminate()
            display.wait(timeout=5)


if __name__ == "__main__":
    main()
