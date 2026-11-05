"""Generated transfer/import fixtures and GTK checks on a private display/bus.

Never installs Hermes or changes the user's desktop, data roots or providers.
Run after `cargo test --locked --bin hermes --no-run` with its test executable.
"""

import argparse
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time


MODULES = [
    "adapters::local_operations", "paths::legacy_import::tests",
    "portal::integration::tests", "services::operations::tests", "app::browser::tests",
    "ui::picker::tests", "ui::picker::folders::tests", "ui::home_dashboard::tests",
]
GTK_TESTS = [
    "ui::picker::tests::private_chooser_returns_file_filters_choices_and_cancels_exactly_once",
    "ui::picker::tests::private_save_requires_overwrite_confirmation_and_closes_child_dialogs",
    "ui::picker::tests::private_new_folder_control_creates_and_enters_folder_without_changing_save_name",
    "ui::theme::tests::picker_tracks_atomic_preferences_new_custom_themes_and_font_changes",
    "ui::browser::conflicts::tests::conflict_dialog_returns_the_chosen_policy_and_cancels_when_dismissed",
    "ui::home_dashboard::tests::private_home_shortcuts_have_two_balanced_rows_and_a_personal_folder_action",
    "ui::legacy_import::tests::private_first_launch_waits_for_explicit_consent_before_importing",
]


def run_case(binary, base, case, index, gui):
    profile = base / f"case-{index:02d}"
    profile.mkdir(mode=0o700)
    env = os.environ.copy()
    for key in ("DBUS_SESSION_BUS_ADDRESS", "DBUS_STARTER_ADDRESS", "DBUS_STARTER_BUS_TYPE",
                "DISPLAY", "WAYLAND_DISPLAY", "HERMES_DEV", "STRATA_DEV",
                "XDG_DESKTOP_PORTAL_DIR", "GTK_THEME", "HERMES_PICKER_SCREENSHOT",
                "HERMES_HOME_SCREENSHOT"):
        env.pop(key, None)
    for key, directory in (("XDG_CONFIG_HOME", "config"), ("XDG_DATA_HOME", "data"),
                           ("XDG_STATE_HOME", "state"), ("XDG_CACHE_HOME", "cache"),
                           ("XDG_RUNTIME_DIR", "runtime"), ("XDG_CONFIG_DIRS", "system-config")):
        path = profile / directory
        path.mkdir(mode=0o700)
        env[key] = str(path)
    env.update(HERMES_SAFETY_TEST_ROOT=str(profile), GDK_BACKEND="broadway",
               BROADWAY_DISPLAY=f":{1000 + (os.getpid() % 20000) + index}", GTK_A11Y="none",
               GTK_USE_PORTAL="0", GDK_DEBUG="no-portals", G_DEBUG="fatal-criticals",
               GSETTINGS_BACKEND="memory", GIO_USE_VFS="local", GIO_USE_VOLUME_MONITOR="unix",
               XDG_CURRENT_DESKTOP="HermesTest")
    if not case.startswith("ui::legacy_import::"):
        env["HERMES_DEV"] = "1"
    folders = []
    for name in ("Desktop", "Documents", "Downloads", "Music", "Pictures", "Videos"):
        path = profile / name
        path.mkdir()
        key = "DOWNLOAD" if name == "Downloads" else name.upper()
        folders.append(f'XDG_{key}_DIR="{path}"')
    (profile / "config/user-dirs.dirs").write_text("\n".join(folders) + "\n")
    bookmarks = []
    for name in ("Wallpapers", "Important", "Favorites", "Screenshots"):
        path = profile / name
        path.mkdir()
        bookmarks.append(f"{path.as_uri()} {name}")
    (profile / "config/gtk-3.0").mkdir()
    (profile / "config/gtk-3.0/bookmarks").write_text("\n".join(bookmarks) + "\n")
    if case == GTK_TESTS[0]:
        env["HERMES_PICKER_SCREENSHOT"] = str(profile / "picker.png")
    if case == GTK_TESTS[5]:
        env["HERMES_HOME_SCREENSHOT"] = str(profile / "home.png")
    with (profile / "display.log").open("w") as display_log:
        server = subprocess.Popen(["gtk4-broadwayd", "-a", "127.0.0.1", "--port", "0",
                                   env["BROADWAY_DISPLAY"]], env=env, stdout=display_log, stderr=display_log)
        try:
            time.sleep(0.15)
            if server.poll() is not None:
                raise RuntimeError(f"Private display failed: {profile / 'display.log'}")
            invocation = [str(binary), case, "--test-threads=1", "--nocapture"]
            if gui:
                invocation += ["--exact", "--ignored"]
            with (profile / "test.log").open("w") as log:
                result = subprocess.run(["dbus-run-session", "--"] + invocation,
                    env=env, stdout=log, stderr=log, timeout=70)
            output = (profile / "test.log").read_text()
            if result.returncode or "running 0 tests" in output or "Theme parser error" in output:
                raise RuntimeError(f"Check failed: {case}\n{output[-7000:]}\nLog: {profile / 'test.log'}")
            print(f"PASS {case}: {next(line for line in output.splitlines() if line.startswith('test result:'))}", flush=True)
        finally:
            server.terminate()
            server.wait(timeout=5)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--test-binary", type=Path, required=True)
    args = parser.parse_args()
    for program in ("gtk4-broadwayd", "dbus-run-session"):
        if not shutil.which(program):
            parser.error(f"Missing test dependency: {program}")
    binary = args.test_binary.resolve(strict=True)
    base = Path(tempfile.mkdtemp(prefix="hermes-release-safety-"))
    print(f"Private test logs and generated profiles: {base}", flush=True)
    for index, case in enumerate(MODULES + GTK_TESTS):
        run_case(binary, base, case, index, case in GTK_TESTS)
    print("Safety/UI checks passed; physical Wayland/browser/audio acceptance remains separate.")


if __name__ == "__main__":
    main()
