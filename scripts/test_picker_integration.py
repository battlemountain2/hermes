import importlib.util
import configparser
import json
import os
import pathlib
import shlex
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


sys.dont_write_bytecode = True
SCRIPT = pathlib.Path(__file__).with_name("desktop-integration.py")
SPEC = importlib.util.spec_from_file_location("picker_integration", SCRIPT)
setup = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(setup)


class PickerSetupTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="hermes-picker-setup-")
        self.root = pathlib.Path(self.temporary.name)
        self.config, self.data, self.state = (self.root / name for name in ("config", "data", "state"))
        self.fallback = self.root / "system-config"
        self.integration = setup.PickerSetup(self.config, self.data, self.state, "Hyprland:GNOME", [self.fallback], [self.root / "system-data"])
        self.binary = self.root / "bin with spaces/hermes"
        self.binary.parent.mkdir()
        self.binary.write_bytes(b"\x7fELFfixtureHERMES_FILE_CHOOSER_V1")
        self.binary.chmod(0o755)
        self.original = b"# preserve providers\r\n[preferred]\r\ndefault=hyprland;kde;gtk;\r\norg.freedesktop.impl.portal.FileChooser=kde\r\norg.freedesktop.impl.portal.ScreenCast=hyprland\r\n"
        self.inherited = self.fallback / "xdg-desktop-portal/hyprland-portals.conf"
        self.seed(self.inherited, self.original)

    def tearDown(self):
        self.temporary.cleanup()

    def seed(self, path, content, mode=0o600):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content)
        path.chmod(mode)

    def test_enable_changes_only_chooser_provider_and_restore_is_exact(self):
        self.seed(self.integration.portal_config, self.original, 0o640)
        untouched = self.config / "mimeapps.list"
        self.seed(untouched, b"[Default Applications]\ninode/directory=dolphin.desktop;\n")
        self.integration.enable(self.binary)
        content = self.integration.portal_config.read_bytes()
        self.assertIn(b"org.freedesktop.impl.portal.FileChooser=hermes\n", content)
        self.assertIn(b"org.freedesktop.impl.portal.ScreenCast=hyprland\r\n", content)
        self.assertIn(b"default=hyprland;kde;gtk;\r\n", content)
        self.assertIn(b"--portal-file-chooser", self.integration.service.read_bytes())
        execution = self.integration.service.read_text().split("Exec=", 1)[1].strip()
        self.assertEqual(shlex.split(execution), ["/usr/bin/env", "GDK_DEBUG=no-portals", "GTK_USE_PORTAL=0", str(self.binary), "--portal-file-chooser"])
        self.assertEqual(self.integration.manifest.read_bytes(), setup.PICKER_MANIFEST)
        self.assertEqual(json.loads(self.integration.preference.read_bytes()), {"version": 1, "enabled": True})
        journal = self.integration.journal.read_bytes()
        self.integration.enable(self.binary)
        self.assertEqual(self.integration.journal.read_bytes(), journal)
        self.integration.restore()
        self.assertEqual(self.integration.portal_config.read_bytes(), self.original)
        self.assertEqual(self.integration.portal_config.stat().st_mode & 0o777, 0o640)
        self.assertFalse(self.integration.service.exists())
        self.assertFalse(self.integration.manifest.exists())
        self.assertFalse(self.integration.preference.exists())
        self.assertEqual(untouched.read_bytes(), b"[Default Applications]\ninode/directory=dolphin.desktop;\n")

    def test_inherited_config_is_copied_not_modified_and_override_removed_on_restore(self):
        self.integration.enable(self.binary)
        self.assertEqual(self.inherited.read_bytes(), self.original)
        self.assertTrue(self.integration.portal_config.exists())
        self.integration.restore()
        self.assertFalse(self.integration.portal_config.exists())
        self.assertEqual(self.inherited.read_bytes(), self.original)

    def test_generic_user_config_beats_system_desktop_config(self):
        generic = self.config / "xdg-desktop-portal/portals.conf"
        self.seed(generic, b"[preferred]\ndefault=gtk\n")
        self.integration.enable(self.binary)
        self.assertIn(b"default=gtk\n", self.integration.portal_config.read_bytes())
        self.integration.restore()
        self.assertEqual(generic.read_bytes(), b"[preferred]\ndefault=gtk\n")

    def test_order_prepares_backend_before_switching_provider_and_restores_provider_first(self):
        journal = self.integration.plan(self.binary)
        paths = [pathlib.Path(change["path"]) for change in self.integration.ordered_changes(journal)]
        self.assertEqual(paths, [self.integration.service, self.integration.preference, self.integration.manifest, self.integration.portal_config])
        self.assertEqual(list(reversed(paths))[0], self.integration.portal_config)

    def test_interrupted_setup_is_recoverable_without_touching_other_portals(self):
        original = setup.write
        def failing(path, content, mode):
            if path == self.integration.manifest:
                raise OSError("Generated failure")
            return original(path, content, mode)
        with mock.patch.object(setup, "write", failing):
            with self.assertRaises(OSError):
                self.integration.enable(self.binary)
        self.assertEqual(self.integration.load()["phase"], "prepared")
        self.assertFalse(self.integration.portal_config.exists())
        self.integration.restore()
        self.assertFalse(self.integration.preference.exists())
        self.assertFalse(self.integration.service.exists())
        self.assertEqual(self.inherited.read_bytes(), self.original)

    def test_external_edits_refuse_partial_restoration(self):
        self.integration.enable(self.binary)
        self.integration.portal_config.write_bytes(b"[preferred]\ndefault=changed\n")
        service = self.integration.service.read_bytes()
        with self.assertRaisesRegex(ValueError, "changed outside"):
            self.integration.restore()
        self.assertEqual(self.integration.service.read_bytes(), service)
        self.assertTrue(self.integration.preference.exists())

    def test_untrusted_journal_cannot_restore_an_unrelated_path(self):
        self.integration.enable(self.binary)
        journal = self.integration.load()
        journal["changes"][0]["path"] = str(self.config / "unrelated")
        self.integration.journal.write_text(json.dumps(journal))
        with self.assertRaisesRegex(ValueError, "unexpected"):
            self.integration.restore()

    def test_bad_configs_custom_discovery_and_incompatible_binary_are_refused(self):
        with self.assertRaises(configparser.Error):
            setup.portal_override(b"[preferred]\ndefault=gtk\ndefault=kde\n")
        with self.assertRaises(ValueError):
            setup.portal_override(b"[other]\nprovider=gtk\n")
        with mock.patch.dict(os.environ, {"XDG_DESKTOP_PORTAL_DIR": "/custom"}):
            with self.assertRaisesRegex(ValueError, "custom"):
                self.integration.plan(self.binary)
        self.binary.write_bytes(b"\x7fELFold")
        with self.assertRaisesRegex(ValueError, "capable"):
            self.integration.plan(self.binary)
        self.assertFalse(self.integration.journal.exists())

    def test_symlinks_and_shared_configuration_are_refused(self):
        self.integration.manifest.parent.mkdir(parents=True)
        self.integration.manifest.symlink_to(self.inherited)
        with self.assertRaises((OSError, ValueError)):
            self.integration.plan(self.binary)
        self.assertEqual(self.inherited.read_bytes(), self.original)

    def test_dry_run_writes_nothing_and_leaves_folder_integration_unchanged(self):
        environment = dict(os.environ, XDG_CONFIG_HOME=str(self.config), XDG_DATA_HOME=str(self.data),
                           XDG_STATE_HOME=str(self.state), XDG_CONFIG_DIRS=str(self.fallback), XDG_CURRENT_DESKTOP="Hyprland")
        environment.pop("XDG_DESKTOP_PORTAL_DIR", None)
        before = sorted(self.root.rglob("*"))
        result = subprocess.run([sys.executable, str(SCRIPT), "enable", "--component", "picker", "--binary", str(self.binary)], env=environment, text=True, capture_output=True, check=True)
        self.assertIn("Preview only", result.stdout)
        self.assertIn("Folder defaults unchanged", result.stdout)
        self.assertEqual(before, sorted(self.root.rglob("*")))

    def test_duplicate_backend_service_is_refused_before_journaling(self):
        duplicate = self.integration.service.with_name("duplicate.service")
        self.seed(duplicate, f"[D-BUS Service]\nName={setup.PICKER_BUS}\nExec=other\n".encode())
        with self.assertRaisesRegex(ValueError, "Another user picker"):
            self.integration.enable(self.binary)
        self.assertFalse(self.integration.journal.exists())


if __name__ == "__main__":
    unittest.main()
