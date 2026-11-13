import importlib.util
import pathlib
import struct
import subprocess
import sys
import tempfile
import unittest
import xml.etree.ElementTree as xml


sys.dont_write_bytecode = True
ROOT = pathlib.Path(__file__).resolve().parents[1]


def module(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / f"{name}.py")
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


installer = module("install-local")
generator = module("generate-app-icons")


class IdentityTests(unittest.TestCase):
    def test_app_and_symbolic_marks_preserve_the_approved_geometry(self):
        paths = []
        for category, suffix in [("scalable", ""), ("symbolic", "-symbolic")]:
            path = ROOT / f"data/icons/{category}/apps/{installer.APP_ID}{suffix}.svg"
            document = xml.parse(path)
            paths.append([node.attrib["d"] for node in document.iter("{http://www.w3.org/2000/svg}path")])
            self.assertEqual(document.getroot().attrib["viewBox"], "0 0 128 128")
            self.assertEqual(len(list(document.iter("{http://www.w3.org/2000/svg}filter"))), 0)
        self.assertEqual(paths[0], paths[1])
        self.assertEqual(len(paths[0]), 2)

    def test_linux_and_mac_icon_dimensions_and_container(self):
        for size in generator.SIZES:
            path = ROOT / f"data/icons/{size}x{size}/apps/{installer.APP_ID}.png"
            data = path.read_bytes()
            self.assertEqual(data[:8], b"\x89PNG\r\n\x1a\n")
            self.assertEqual(struct.unpack(">II", data[16:24]), (size, size))
            self.assertEqual(data[25], 6)
        data = (ROOT / "packaging/macos/Hermes.icns").read_bytes()
        self.assertEqual(data[:4], b"icns")
        self.assertEqual(struct.unpack(">I", data[4:8])[0], len(data))
        position = 8
        entries = {}
        while position < len(data):
            kind, size = struct.unpack(">4sI", data[position:position + 8])
            self.assertGreater(size, 8)
            entries[kind] = data[position + 8:position + size]
            position += size
        self.assertEqual(position, len(data))
        for kind, size in generator.ICNS_SIZES.items():
            self.assertEqual(struct.unpack(">II", entries[kind][16:24]), (size, size))

    def test_isolated_install_uses_absolute_launcher_and_preserves_strata(self):
        with tempfile.TemporaryDirectory(prefix="hermes-install-test-") as folder:
            root = pathlib.Path(folder)
            binary = root / "built-hermes"
            binary.write_bytes(b"generated installation fixture")
            prefix = root / "prefix with spaces"
            old = prefix / "bin/strata"
            old.parent.mkdir(parents=True)
            old.write_bytes(b"unrelated upstream app")
            installer.install(ROOT, binary, prefix, prefix / "share")
            target = prefix / "bin/hermes"
            self.assertEqual(target.read_bytes(), binary.read_bytes())
            self.assertEqual(target.stat().st_mode & 0o777, 0o755)
            launcher = prefix / "share/applications" / f"{installer.APP_ID}.desktop"
            self.assertIn(f'Exec="{target}" %U', launcher.read_text())
            subprocess.run(["desktop-file-validate", str(launcher)], check=True)
            development = launcher.with_name(f"{installer.APP_ID}.Dev.desktop")
            self.assertIn("NoDisplay=true", development.read_text())
            self.assertIn(f'Exec=env HERMES_DEV=1 "{target}" %U', development.read_text())
            subprocess.run(["desktop-file-validate", str(development)], check=True)
            self.assertEqual(old.read_bytes(), b"unrelated upstream app")
            self.assertFalse((prefix / "share/applications/mimeapps.list").exists())
            self.assertFalse((prefix / "share/dbus-1").exists())

    def test_existing_files_and_symlinks_are_not_silently_replaced(self):
        with tempfile.TemporaryDirectory(prefix="hermes-install-test-") as folder:
            root = pathlib.Path(folder)
            binary = root / "binary"
            binary.write_bytes(b"new")
            prefix = root / "prefix"
            target = prefix / "bin/hermes"
            target.parent.mkdir(parents=True)
            target.write_bytes(b"old")
            with self.assertRaises(FileExistsError):
                installer.install(ROOT, binary, prefix, prefix / "share")
            self.assertEqual(target.read_bytes(), b"old")
            target.unlink()
            target.symlink_to(binary)
            with self.assertRaises(ValueError):
                installer.install(ROOT, binary, prefix, prefix / "share", replace=True)
            self.assertEqual(binary.read_bytes(), b"new")

    def test_desktop_exec_preserves_spaces_and_percent_and_rejects_ambiguous_paths(self):
        self.assertEqual(installer.desktop_exec(pathlib.Path("/tmp/50% complete/hermes")), '"/tmp/50%% complete/hermes" %U')
        for path in ["/tmp/line\nbreak/hermes", '/tmp/"/hermes', "/tmp/`/hermes", "/tmp/$/hermes", "/tmp/\\/hermes"]:
            with self.assertRaises(ValueError):
                installer.desktop_exec(pathlib.Path(path))


if __name__ == "__main__":
    unittest.main()
