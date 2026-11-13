import importlib.util
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


sys.dont_write_bytecode = True
SCRIPT = pathlib.Path(__file__).with_name("desktop-integration.py")
SPEC = importlib.util.spec_from_file_location("desktop_integration", SCRIPT)
setup = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(setup)


class FolderSetupTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="hermes-desktop-test-")
        self.root = pathlib.Path(self.temporary.name)
        self.config, self.data, self.state = (self.root / name for name in ("config", "data", "state"))
        self.integration = setup.Setup(self.config, self.data, self.state)
        self.binary = self.root / "bin with spaces/hermes"
        self.binary.parent.mkdir()
        self.binary.write_bytes(b"\x7fELFfixture" + setup.CAPABILITY)
        self.binary.chmod(0o755)
        self.launcher = self.data / "applications" / f"{setup.APP_ID}.desktop"
        self.launcher.parent.mkdir(parents=True)
        self.launcher.write_text(f'[Desktop Entry]\nType=Application\nName=Hermes\nExec="{self.binary}" %U\nMimeType=inode/directory;\n')

    def tearDown(self):
        self.temporary.cleanup()

    def seed(self, path, content, mode=0o600):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content)
        path.chmod(mode)

    def test_enable_is_idempotent_and_restore_is_byte_exact(self):
        originals = [(self.integration.mime, b"# settings\r\n[Default Applications]\r\ninode/directory=org.kde.dolphin.desktop;\r\nimage/png=photo.desktop;\r\n", 0o640),
                     (self.integration.service, b"[D-BUS Service]\nName=org.freedesktop.FileManager1\nExec=previous-manager\n", 0o644),
                     (self.integration.preference, b'{"version":1,"enabled":false}\n', 0o600)]
        for path, content, mode in originals:
            self.seed(path, content, mode)
        self.integration.enable(self.binary)
        state = self.integration.journal.read_bytes()
        self.integration.enable(self.binary)
        self.assertEqual(self.integration.journal.read_bytes(), state)
        self.assertEqual(self.integration.load()["phase"], "enabled")
        self.assertIn(f"inode/directory={setup.APP_ID}.desktop;", self.integration.mime.read_text())
        self.assertIn(f"Exec='{self.binary}' --gapplication-service", self.integration.service.read_text())
        self.assertIn(b'"enabled":true', self.integration.preference.read_bytes())
        self.assertEqual(self.integration.journal.stat().st_mode & 0o777, 0o600)
        self.integration.restore()
        for path, content, mode in originals:
            self.assertEqual(path.read_bytes(), content)
            self.assertEqual(path.stat().st_mode & 0o777, mode)
        self.assertFalse(self.integration.journal.exists())
        self.integration.restore()

    def test_new_files_are_removed_and_inherited_defaults_are_left_alone(self):
        generic = self.config / "mimeapps.list"
        self.seed(generic, b"[Default Applications]\ninode/directory=org.kde.dolphin.desktop;\n")
        integration = setup.Setup(self.config, self.data, self.state, "Hyprland:GNOME")
        original = generic.read_bytes()
        integration.enable(self.binary)
        self.assertEqual(integration.mime.name, "hyprland-mimeapps.list")
        self.assertEqual(generic.read_bytes(), original)
        integration.restore()
        for path in (integration.mime, integration.service, integration.preference):
            self.assertFalse(path.exists())
        self.assertEqual(generic.read_bytes(), original)

    def test_external_edits_abort_restoration_before_any_other_file_is_touched(self):
        self.integration.enable(self.binary)
        service = self.integration.service.read_bytes()
        preference = self.integration.preference.read_bytes()
        self.integration.mime.write_bytes(b"[Default Applications]\ninode/directory=other.desktop;\n")
        with self.assertRaisesRegex(ValueError, "changed outside"):
            self.integration.restore()
        self.assertEqual(self.integration.service.read_bytes(), service)
        self.assertEqual(self.integration.preference.read_bytes(), preference)
        self.assertTrue(self.integration.journal.exists())

    def test_changed_permissions_are_not_silently_overwritten(self):
        self.integration.enable(self.binary)
        self.integration.mime.chmod(0o644)
        with self.assertRaisesRegex(ValueError, "changed outside"):
            self.integration.restore()
        self.assertTrue(self.integration.preference.exists())

    def test_interrupted_enable_retains_a_recoverable_journal(self):
        original = setup.write
        def failing(path, content, mode):
            if path == self.integration.service:
                raise OSError("Generated write failure")
            return original(path, content, mode)
        with mock.patch.object(setup, "write", failing):
            with self.assertRaises(OSError):
                self.integration.enable(self.binary)
        self.assertEqual(self.integration.load()["phase"], "prepared")
        with self.assertRaisesRegex(ValueError, "interrupted"):
            self.integration.enable(self.binary)
        self.integration.restore()
        self.assertFalse(self.integration.mime.exists())
        self.assertFalse(self.integration.journal.exists())

    def test_unsafe_files_and_parent_symlinks_are_refused(self):
        self.config.mkdir()
        external = self.root / "external"
        external.write_bytes(b"keep")
        self.integration.mime.symlink_to(external)
        with self.assertRaises(OSError):
            self.integration.plan(self.binary)
        self.assertEqual(external.read_bytes(), b"keep")
        self.integration.mime.unlink()
        self.integration.mime.hardlink_to(external)
        with self.assertRaises(ValueError):
            self.integration.plan(self.binary)
        self.integration.mime.unlink()
        self.config.rmdir()
        self.config.symlink_to(self.root)
        with self.assertRaises(ValueError):
            self.integration.plan(self.binary)

    def test_conflicting_user_activation_and_old_binaries_are_refused(self):
        conflict = self.integration.service.with_name("other-manager.service")
        self.seed(conflict, b"[D-BUS Service]\nName=org.freedesktop.FileManager1\nExec=other\n")
        with self.assertRaisesRegex(ValueError, "Another user reveal"):
            self.integration.plan(self.binary)
        conflict.unlink()
        self.binary.write_bytes(b"\x7fELFold build")
        with self.assertRaisesRegex(ValueError, "capable"):
            self.integration.plan(self.binary)
        self.assertFalse(self.integration.journal.exists())

    def test_launcher_must_match_chosen_binary_and_folder_type(self):
        self.launcher.write_text("[Desktop Entry]\nExec=other %U\nMimeType=inode/directory;\n")
        with self.assertRaisesRegex(ValueError, "launcher"):
            self.integration.plan(self.binary)
        self.assertFalse(self.config.exists())

    def test_dry_run_writes_nothing_and_never_launches_the_binary(self):
        environment = dict(os.environ, XDG_CONFIG_HOME=str(self.config), XDG_DATA_HOME=str(self.data),
                           XDG_STATE_HOME=str(self.state), XDG_CURRENT_DESKTOP="Hyprland")
        before = sorted(self.root.rglob("*"))
        result = subprocess.run([sys.executable, str(SCRIPT), "enable", "--binary", str(self.binary)],
                                env=environment, text=True, capture_output=True, check=True)
        self.assertIn("Preview only", result.stdout)
        self.assertIn("Open/Save picker unchanged", result.stdout)
        self.assertEqual(before, sorted(self.root.rglob("*")))

    def test_journal_cannot_restore_arbitrary_paths(self):
        self.integration.enable(self.binary)
        journal = self.integration.load()
        journal["changes"][0]["path"] = str(self.root / "unrelated")
        self.integration.journal.write_text(json.dumps(journal))
        with self.assertRaisesRegex(ValueError, "unexpected"):
            self.integration.restore()
        self.assertTrue(self.integration.preference.exists())

    def test_lock_prevents_competing_writers(self):
        with self.integration.lock():
            with self.assertRaisesRegex(ValueError, "Another Hermes"):
                self.integration.lock()

    def test_folder_key_edit_preserves_other_entries_and_rejects_duplicates(self):
        for content in [b"# only a comment", b"[Default Applications]", b"[Other]\na=b\n"]:
            output = setup.folder_default(content)
            self.assertIn(f"inode/directory={setup.APP_ID}.desktop;".encode(), output)
        with self.assertRaises(ValueError):
            setup.folder_default(b"[Default Applications]\ninode/directory=a;\ninode/directory=b;\n")
        with self.assertRaises(ValueError):
            setup.folder_default(b"[Default Applications]\n[Default Applications]\n")

    def test_activation_refresh_only_calls_the_existing_bus_daemon(self):
        with mock.patch.dict(os.environ, {"DBUS_SESSION_BUS_ADDRESS": "unix:path=/generated/private-bus"}):
            with mock.patch.object(setup.subprocess, "run", return_value=mock.Mock(returncode=0)) as run:
                self.assertTrue(setup.refresh_activation())
                arguments = run.call_args.args[0]
                self.assertEqual(arguments[arguments.index("--dest") + 1], "org.freedesktop.DBus")
                self.assertEqual(arguments[-1], "org.freedesktop.DBus.ReloadConfig")
            with mock.patch.object(setup.subprocess, "run", side_effect=OSError):
                self.assertFalse(setup.refresh_activation())


if __name__ == "__main__":
    unittest.main()
