import importlib.util
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest


SCRIPT = pathlib.Path(__file__).with_name("profile-fixture.py")
sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("profile_fixture", SCRIPT)
profile = importlib.util.module_from_spec(spec)
spec.loader.exec_module(profile)


class ProfileTests(unittest.TestCase):
    def test_current_process_is_in_memory_tree(self):
        self.assertIn(os.getpid(), profile.descendants(os.getpid()))
        rss, pss = profile.sample_memory(os.getpid())
        self.assertGreater(rss, 0)
        self.assertGreater(pss, 0)

    def test_probe_isolates_state_and_reports_incomplete_runs(self):
        with tempfile.TemporaryDirectory(prefix="hermes-profile-test.") as folder:
            root = pathlib.Path(folder)
            binary = root / "fake"
            binary.write_text("#!/usr/bin/env python3\nimport os,time,pathlib\npathlib.Path(os.environ['XDG_STATE_HOME'],'marker').write_text('fixture')\nprint('directory load finished',flush=True)\ntime.sleep(30)\n")
            binary.chmod(0o700)
            env = os.environ.copy()
            env.pop("STRATA_PROFILE_HEADLESS", None)
            env.pop("HERMES_PROFILE_HEADLESS", None)
            env["XDG_STATE_HOME"] = str(root / "personal")
            run = subprocess.run(["python3", str(SCRIPT), str(root), "2", "--binary", str(binary)], env=env, capture_output=True, text=True, timeout=15)
            self.assertEqual(run.returncode, 0, run.stderr)
            self.assertIn("process-tree peak PSS", run.stdout)
            self.assertFalse((root / "personal").exists())
            pathlib.Path(run.stdout.split("Full log: ")[-1].strip()).unlink()
            binary.write_text("#!/usr/bin/env python3\nimport time\ntime.sleep(30)\n")
            run = subprocess.run(["python3", str(SCRIPT), str(root), ".1", "--binary", str(binary)], env=env, capture_output=True, text=True, timeout=15)
            self.assertNotEqual(run.returncode, 0)
            self.assertIn("Profile incomplete", run.stderr)
            pathlib.Path(run.stdout.split("Full log: ")[-1].strip()).unlink()


if __name__ == "__main__":
    unittest.main()
