import argparse
import importlib.util
import os
import signal
from pathlib import Path
import subprocess
import sys
import tempfile
import time
from xml.sax.saxutils import escape


sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[1]
FRONTEND = "org.freedesktop.portal.Desktop"
BACKEND = "org.freedesktop.impl.portal.desktop.hermes"
PATH = "/org/freedesktop/portal/desktop"


def check_private(root, binary, test_binary):
    if (os.environ.get("HERMES_PICKER_TEST_ROOT") != str(root)
            or os.environ.get("HOME") != str(root / "home")
            or os.environ.get("XDG_CONFIG_HOME") != str(root / "config")
            or os.environ.get("XDG_DATA_HOME") != str(root / "data")
            or os.environ.get("GDK_BACKEND") != "broadway"):
        raise RuntimeError("Use the parent command to create an isolated display and session")
    if test_binary:
        for test in ["ui::picker::tests::private_chooser_returns_file_filters_choices_and_cancels_exactly_once",
                     "ui::picker::visuals::tests::private_picker_renders_folder_and_selected_previews_without_stale_images",
                     "ui::browser::app_chooser::tests::suggested_apps_precede_unrelated_apps_without_launching_or_changing_defaults",
                     "ui::picker::tests::private_save_requires_overwrite_confirmation_and_closes_child_dialogs",
                     "portal::tests::private_bus_authenticates_frontend_closes_requests_and_returns_selected_files"]:
            unit_environment = dict(os.environ)
            unit_environment.pop("GDK_DEBUG", None)
            if test.startswith("ui::picker::visuals::"):
                unit_environment["G_DEBUG"] = "fatal-criticals"
            subprocess.run([str(test_binary), test, "--ignored", "--exact", "--test-threads=1", "--nocapture"], env=unit_environment, check=True, timeout=50)
    spec = importlib.util.spec_from_file_location("picker_setup", ROOT / "scripts/desktop-integration.py")
    setup = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(setup)
    baseline = root / "config/xdg-desktop-portal/portals.conf"
    baseline.parent.mkdir(parents=True, exist_ok=True)
    baseline.write_bytes(b"[preferred]\ndefault=none\norg.freedesktop.impl.portal.FileChooser=none\n")
    original = baseline.read_bytes()
    integration = setup.PickerSetup(root / "config", root / "data", root / "state", "HermesTest")
    with integration.lock():
        integration.enable(binary)
    assert setup.refresh_activation()
    import gi
    gi.require_version("Gio", "2.0")
    from gi.repository import Gio, GLib
    connection = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    context = GLib.MainContext.default()
    def call(destination, path, interface, method, arguments, reply_type=None):
        return connection.call_sync(destination, path, interface, method, arguments, reply_type,
                                    Gio.DBusCallFlags.NONE, 5000, None)
    def until(condition):
        deadline = time.monotonic() + 10
        while not condition():
            if time.monotonic() > deadline:
                raise AssertionError("Timed out waiting for private frontend/chooser state")
            context.iteration(False)
            time.sleep(0.01)
    def stop_private_backend():
        owned = call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "NameHasOwner", GLib.Variant("(s)", (BACKEND,))).unpack()[0]
        if not owned:
            return
        process = call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "GetConnectionUnixProcessID", GLib.Variant("(s)", (BACKEND,))).unpack()[0]
        environment = dict(item.split(b"=", 1) for item in Path(f"/proc/{process}/environ").read_bytes().split(b"\0") if b"=" in item)
        assert environment.get(b"HERMES_PICKER_TEST_ROOT") == str(root).encode(), "Refusing to stop a non-fixture process"
        os.kill(process, signal.SIGTERM)
        until(lambda: not call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "NameHasOwner", GLib.Variant("(s)", (BACKEND,))).unpack()[0])
    timings = []
    for attempt in range(8):
        stop_private_backend()
        started = time.monotonic()
        version = call(BACKEND, PATH, "org.freedesktop.DBus.Properties", "Get", GLib.Variant("(ss)", (setup.PICKER_INTERFACE, "version"))).unpack()[0]
        assert version == 4, "First request after activation must find an already exported chooser"
        timings.append(round((time.monotonic() - started) * 1000))
    stop_private_backend()
    print(f"PASS: eight first-call cold activations, no retry; ready_ms={timings}", flush=True)
    frontend = subprocess.Popen(["/usr/lib/xdg-desktop-portal", "--verbose"])
    try:
        until(lambda: call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "NameHasOwner", GLib.Variant("(s)", (FRONTEND,))).unpack()[0])
        introspection = call(FRONTEND, PATH, "org.freedesktop.DBus.Introspectable", "Introspect", None).unpack()[0]
        assert "org.freedesktop.portal.FileChooser" in introspection, "Frontend did not discover the user-local Hermes manifest"
        for method, options in [("OpenFile", {}), ("SaveFile", {"current_name": GLib.Variant("s", "generated.txt")}),
                                ("SaveFiles", {"files": GLib.Variant("aay", [list(b"one.txt\0"), list(b"two.txt\0")])})]:
            options["handle_token"] = GLib.Variant("s", "hermescheck_" + method.lower())
            request = call(FRONTEND, PATH, "org.freedesktop.portal.FileChooser", method,
                GLib.Variant("(ssa{sv})", ("", "Generated Hermes chooser", options))).unpack()[0]
            until(lambda: call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "NameHasOwner", GLib.Variant("(s)", (BACKEND,))).unpack()[0])
            version = call(BACKEND, PATH, "org.freedesktop.DBus.Properties", "Get",
                GLib.Variant("(ss)", (setup.PICKER_INTERFACE, "version"))).unpack()[0]
            assert version == 4
            process = call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "GetConnectionUnixProcessID", GLib.Variant("(s)", (BACKEND,))).unpack()[0]
            backend_environment = dict(item.split(b"=", 1) for item in Path(f"/proc/{process}/environ").read_bytes().split(b"\0") if b"=" in item)
            assert backend_environment[b"GDK_DEBUG"] == b"no-portals", "Backend can recursively wait for its frontend during GTK startup"
            def forwarded():
                try:
                    xml = call(BACKEND, request, "org.freedesktop.DBus.Introspectable", "Introspect", None).unpack()[0]
                    return "org.freedesktop.impl.portal.Request" in xml
                except GLib.Error:
                    return False
            until(forwarded)
            call(FRONTEND, request, "org.freedesktop.portal.Request", "Close", None)
            until(lambda: not forwarded())
        with integration.lock():
            integration.restore()
        assert setup.refresh_activation()
        until(lambda: not call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "NameHasOwner", GLib.Variant("(s)", (BACKEND,))).unpack()[0])
        assert baseline.read_bytes() == original
        assert not integration.portal_config.exists()
        assert not integration.preference.exists()
        assert not integration.service.exists()
        assert not integration.manifest.exists()
        print("PASS: real private frontend discovers user-local manifest, activates Hermes, forwards OpenFile/SaveFile/SaveFiles/Close and releases opt-in; previous providers restored", flush=True)
    finally:
        frontend.terminate()
        frontend.wait(timeout=5)


def main():
    parser = argparse.ArgumentParser(description="Validate Hermes Open/Save on a generated private D-Bus/Broadway/XDG session; never changes the real desktop")
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--test-binary", type=Path)
    parser.add_argument("--private-root", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    test_binary = args.test_binary.resolve(strict=True) if args.test_binary else None
    if args.private_root:
        check_private(args.private_root, binary, test_binary)
        return
    root = Path(tempfile.mkdtemp(prefix="hermes-picker-runtime-"))
    environment = os.environ.copy()
    for key in ["DBUS_SESSION_BUS_ADDRESS", "DBUS_STARTER_ADDRESS", "DBUS_STARTER_BUS_TYPE", "WAYLAND_DISPLAY", "DISPLAY", "HERMES_DEV", "STRATA_DEV", "XDG_DESKTOP_PORTAL_DIR"]:
        environment.pop(key, None)
    for key, directory in [("HOME", "home"), ("XDG_CONFIG_HOME", "config"), ("XDG_DATA_HOME", "data"),
                           ("XDG_STATE_HOME", "state"), ("XDG_CACHE_HOME", "cache"), ("XDG_RUNTIME_DIR", "runtime")]:
        path = root / directory
        path.mkdir(mode=0o700)
        environment[key] = str(path)
    services = root / "data/dbus-1/services"
    services.mkdir(parents=True)
    environment.update(HERMES_PICKER_TEST_ROOT=str(root), GDK_BACKEND="broadway", BROADWAY_DISPLAY=":3414",
                       GTK_A11Y="none", GTK_USE_PORTAL="0", GDK_DEBUG="portals", GIO_USE_VFS="local", GIO_USE_VOLUME_MONITOR="unix",
                       GSETTINGS_BACKEND="memory", XDG_CURRENT_DESKTOP="HermesTest", XDG_CONFIG_DIRS=str(root / "system-config"),
                       XDG_DATA_DIRS="/usr/share", RUST_LOG="hermes=debug", LC_ALL="C.UTF-8",
                       HERMES_PICKER_SCREENSHOT=str(root / "chooser.png"))
    configuration = root / "bus.conf"
    configuration.write_text(
        '<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN" "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">\n'
        '<busconfig><type>session</type><listen>unix:tmpdir=/tmp</listen><auth>EXTERNAL</auth>'
        f'<servicedir>{escape(str(services))}</servicedir>'
        f'<policy user="{os.getuid()}"><allow own="*"/><allow send_destination="*"/><allow receive_sender="*"/></policy></busconfig>')
    print(f"Private picker validation logs: {root}", flush=True)
    with (root / "runtime.log").open("w") as log:
        display = subprocess.Popen(["gtk4-broadwayd", "-a", "127.0.0.1", "--port", "0", ":3414"], env=environment, stdout=log, stderr=log)
        try:
            time.sleep(0.15)
            arguments = ["dbus-run-session", "--config-file", str(configuration), "--", sys.executable,
                         __file__, "--private-root", str(root), "--binary", str(binary)]
            if test_binary:
                arguments.extend(["--test-binary", str(test_binary)])
            result = subprocess.run(arguments, env=environment, stdout=log, stderr=log, timeout=140)
            if result.returncode:
                raise RuntimeError(f"Private picker check failed; see {root / 'runtime.log'}")
            print("Private picker integration passed; actual defaults, portals and windows unchanged.")
        finally:
            display.terminate()
            display.wait(timeout=5)


if __name__ == "__main__":
    main()
