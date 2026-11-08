"""Validate generated migration fixtures without a desktop or package manager."""

import hashlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import qualify_installed_migration as migration


class MigrationFixtureTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="hermes-migration-fixtures-")
        self.addCleanup(self.temporary.cleanup)
        self.profile = Path(self.temporary.name)

    def test_generated_collections_and_session_reference_the_generated_song(self):
        content = migration.seed(self.profile)
        collections = json.loads(content["data/music-collections-v1.json"])
        queue = json.loads(content["state/music-queue-v1.json"])
        location = collections["favorites"][0]
        self.assertEqual(location, collections["playlists"][0]["tracks"][0])
        self.assertEqual(location, queue["tracks"][0]["path"])
        self.assertEqual(Path(bytes(location["value"]).decode()), self.profile / "Documents/song.wav")
        migration.check_originals(self.profile, content)

    def test_checkpoint_has_private_verifiable_backups_and_no_destination_data(self):
        content = migration.seed(self.profile)
        original = migration.checkpoint(self.profile, content)
        state = migration.journal(self.profile)
        self.assertFalse(state["completed"])
        for index, (name, value) in enumerate(content.items()):
            entry = state["entries"][index]
            backup = self.profile / f"state/hermes/legacy-import-v2/backups/{index:03}.backup"
            self.assertEqual(entry["state"], "BackedUp")
            self.assertEqual(entry["sha256"], hashlib.sha256(value).hexdigest())
            self.assertEqual(backup.read_bytes(), value)
            self.assertEqual(backup.stat().st_mode & 0o777, 0o600)
            self.assertFalse(migration.target_path(self.profile, name).exists())
        self.assertEqual(original, (self.profile / "state/hermes/legacy-import-v2/journal-0000.json").read_bytes())

    def test_original_mutation_is_detected(self):
        content = migration.seed(self.profile)
        migration.source_path(self.profile, "data/ratings-v1.json").write_bytes(b"changed")
        with self.assertRaisesRegex(RuntimeError, "original changed"):
            migration.check_originals(self.profile, content)

    def test_installation_guard_runs_before_gui_or_filesystem_setup(self):
        output = self.profile / "must-not-be-created"
        with patch("sys.argv", ["qualify_installed_migration.py", "--output", str(output),
                                 "--source-commit", "a" * 40]), \
             patch.object(migration, "require_disposable_root", side_effect=ValueError("private root required")), \
             self.assertRaisesRegex(ValueError, "private root"):
            migration.main()
        self.assertFalse(output.exists())

    def test_short_source_identity_is_rejected_before_app_execution(self):
        with patch("sys.argv", ["qualify_installed_migration.py", "--output", str(self.profile / "unused"),
                                 "--source-commit", "abc123"]), \
             patch("sys.stderr", io.StringIO()), \
             patch.object(migration, "require_disposable_root") as guard, self.assertRaises(SystemExit):
            migration.main()
        guard.assert_not_called()


if __name__ == "__main__":
    unittest.main()
