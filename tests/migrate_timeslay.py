"""Run with python3 tests/migrate_timeslay.py; uses isolated home directories."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


class MigrationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.home = Path(self.temp.name)
        self.repo = self.home / "repo"
        self.script = self.repo / "scripts/migrate-timeslay.sh"
        self.script.parent.mkdir(parents=True)
        shutil.copy2(Path(__file__).resolve().parents[1] / "scripts/migrate-timeslay.sh", self.script)
        self.env = dict(os.environ, HOME=str(self.home),
                        XDG_CONFIG_HOME=str(self.home / "config"),
                        XDG_DATA_HOME=str(self.home / "data"),
                        XDG_STATE_HOME=str(self.home / "state"),
                        XDG_CACHE_HOME=str(self.home / "cache"),
                        DBUS_SESSION_BUS_ADDRESS="unix:path=" + str(self.home / "no-bus"))
        self.prefix = self.home / 'prefix with spaces'
        for directory in ("config", "data", "state", "cache"):
            path = self.home / directory / "time-slay"
            path.mkdir(parents=True)
            (path / "fixture").write_text(directory)
        tokens = self.home / "config/time-slay/slack/.slack-app-tokens.json"
        tokens.parent.mkdir()
        tokens.write_text('current rotating token')
        tokens.chmod(0o600)
        os.utime(tokens, (1700000000, 1700000000))
        extension = self.home / "data/timeslay/time-slay-webextension"
        extension.parent.mkdir()
        extension.write_text("old wrapper")
        self.write("target/debug/timeslay", '#!/bin/sh\necho "timeslay 0.1.0"\n', executable=True)
        self.write("packaging/timeslay.desktop.in", "fixture")
        self.write("src/panel/hotkey.rs", 'pub const COMPONENT: &str = "time-slay";')
        self.write("packaging/install.sh", '''#!/bin/bash
set -eu
[[ $1 == --prefix ]]
mkdir -p "$2/bin"
cp "$(dirname "$0")/../target/debug/timeslay" "$2/bin/timeslay"
''', executable=True)

    def write(self, relative, text, executable=False):
        path = self.repo / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
        if executable:
            path.chmod(0o755)

    def run_script(self, *args):
        return subprocess.run(["bash", str(self.script), "--prefix", str(self.prefix), *args],
                              env=self.env, capture_output=True, text=True)

    def test_preview_does_not_change_data(self):
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.home / "config/time-slay/fixture").exists())
        self.assertFalse(list(self.home.glob("timeslay-migration-backup.*")))

    def test_migration_preserves_tokens_and_extension(self):
        result = self.run_script("--apply")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        for directory in ("config", "data", "state", "cache"):
            self.assertEqual((self.home / directory / "timeslay/fixture").read_text(), directory)
            self.assertFalse((self.home / directory / "time-slay").exists())
        tokens = self.home / "config/timeslay/slack/.slack-app-tokens.json"
        self.assertEqual(tokens.read_text(), "current rotating token")
        self.assertEqual(tokens.stat().st_mtime, 1700000000)
        self.assertEqual(tokens.stat().st_mode & 0o777, 0o600)
        self.assertEqual((self.home / "data/timeslay/time-slay-webextension").read_text(), "old wrapper")
        backup, = self.home.glob("timeslay-migration-backup.*")
        self.assertEqual(backup.stat().st_mode & 0o777, 0o700)
        self.assertEqual((backup / "data-0-time-slay/slack/.slack-app-tokens.json").read_text(), tokens.read_text())
        self.assertEqual((self.prefix / "bin/time-slay").resolve(), self.prefix / "bin/timeslay")
        # A repeat does not overwrite the migrated data.
        result = self.run_script("--apply")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(tokens.read_text(), "current rotating token")

    def test_conflict_aborts_without_changes(self):
        conflict = self.home / "config/timeslay"
        conflict.mkdir()
        (conflict / "fixture").write_text("new data")
        result = self.run_script("--apply")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual((conflict / "fixture").read_text(), "new data")
        self.assertTrue((self.home / "config/time-slay/fixture").exists())
        self.assertFalse(list(self.home.glob("timeslay-migration-backup.*")))

    def test_missing_build_aborts_before_backup(self):
        (self.repo / "target/debug/timeslay").unlink()
        result = self.run_script("--apply")
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue((self.home / "config/time-slay/fixture").exists())
        self.assertFalse(list(self.home.glob("timeslay-migration-backup.*")))


if __name__ == "__main__":
    unittest.main()
