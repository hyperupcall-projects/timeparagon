"""Exercise installer lifecycle in an isolated home layout with stubbed tools."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


class InstallTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="timeslay-install-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / "repo"
        packaging = self.repo / "packaging"
        packaging.mkdir(parents=True)
        for source in Path(__file__).resolve().parent.iterdir():
            if source.suffix in (".sh", ".in"):
                shutil.copy2(source, packaging / source.name)
        fixtures = {
            "target/debug/timeslay": "#!/bin/sh\nexit 0\n",
            "assets/noto-hourglass-done.png": "fixture",
            "slack/package.json": '{"private":true}',
            "slack/package-lock.json": '{}',
            "slack/src/app.ts": "// fixture",
            "slack/.env.example": "SLACK_CLIENT_ID=\n",
            "slack/.env": "SLACK_CLIENT_ID=fixture\n",
            "slack/.slack-app-tokens.json": "current-pair",
            "slack/.slack-installations.json": "connected-workspaces",
        }
        for name, value in fixtures.items():
            path = self.repo / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(value)
        (self.repo / "target/debug/timeslay").chmod(0o755)
        stubs = self.root / "tools"
        stubs.mkdir()
        for name in ("node", "npm", "systemctl", "kwriteconfig6", "qdbus6",
                     "update-desktop-database", "gtk-update-icon-cache"):
            path = stubs / name
            path.write_text(
                '#!/bin/sh\n'
                f'printf "%s\\n" "{name} $*" >> "$INSTALL_TEST_LOG"\n'
                'case " $* " in *" is-active "*) exit 1;; esac\nexit 0\n'
            )
            path.chmod(0o755)
        # Exercise systemd quoting and sed substitutions in installation paths.
        self.data = self.root / 'data with spaces % & " |'
        self.config = self.root / "config"
        self.prefix = self.root / "prefix"
        self.log = self.root / "commands.log"
        self.env = {
            **os.environ, "PATH": f"{stubs}:{os.environ['PATH']}",
            "XDG_DATA_HOME": str(self.data), "XDG_CONFIG_HOME": str(self.config),
            "INSTALL_TEST_LOG": str(self.log),
        }
        self.slack_config = self.config / "timeslay/slack"
        self.unit = self.config / "systemd/user/timeslay-slack.service"

    def run_script(self, name, *args):
        return subprocess.run(
            ["sh", str(self.repo / "packaging" / name), "--prefix", str(self.prefix), *args],
            env=self.env, check=True, capture_output=True, text=True,
        )

    def test_install_upgrade_and_uninstall(self):
        self.run_script("install.sh", "--no-build")
        self.assertTrue((self.data / "timeslay/slack/src/app.ts").exists())
        self.assertIn("npm ci --omit=dev", self.log.read_text())
        self.assertIn("systemctl --user enable timeslay-slack.service", self.log.read_text())
        self.assertIn("systemctl --user restart timeslay-slack.service", self.log.read_text())
        installations = self.slack_config / ".slack-installations.json"
        self.assertEqual(installations.read_text(), "connected-workspaces")
        self.assertEqual(installations.stat().st_mode & 0o777, 0o600)
        self.assertIn("SLACK_INSTALLATIONS_FILE=", self.unit.read_text())
        self.assertNotIn("@INSTALLATIONS_FILE@", self.unit.read_text())
        tokens = self.slack_config / ".slack-app-tokens.json"
        env_file = self.slack_config / ".env"
        self.assertEqual(tokens.read_text(), "current-pair")
        self.assertEqual(tokens.stat().st_mode & 0o777, 0o600)
        self.assertEqual(env_file.stat().st_mode & 0o777, 0o600)
        if shutil.which("systemd-analyze"):
            checked = subprocess.run(["systemd-analyze", "--user", "verify", str(self.unit)],
                                     capture_output=True, text=True)
            self.assertEqual(checked.returncode, 0, checked.stderr)
        installations.write_text("newer-workspaces")
        tokens.write_text("newer-pair")
        env_file.write_text("updated-credentials")
        self.run_script("install.sh", "--no-build")
        self.assertEqual(tokens.read_text(), "newer-pair")
        self.assertEqual(installations.read_text(), "newer-workspaces")
        self.assertEqual(env_file.read_text(), "updated-credentials")
        self.run_script("uninstall.sh")
        self.assertFalse(self.unit.exists())
        self.assertFalse((self.data / "timeslay/slack").exists())
        self.assertEqual(tokens.read_text(), "newer-pair")
        self.assertEqual(installations.read_text(), "newer-workspaces")
        self.run_script("uninstall.sh", "--purge")
        self.assertFalse(self.slack_config.exists())

    def test_no_enable(self):
        self.run_script("install.sh", "--no-build", "--no-enable")
        self.assertTrue(self.unit.exists())
        commands = self.log.read_text()
        self.assertNotIn("systemctl --user enable ", commands)
        self.assertNotIn("systemctl --user restart ", commands)

    def test_unconfigured_install_waits_for_credentials(self):
        (self.repo / "slack/.env").unlink()
        (self.repo / "slack/.slack-app-tokens.json").unlink()
        result = self.run_script("install.sh", "--no-build")
        self.assertIn("Slack setup:", result.stdout)
        self.assertFalse((self.slack_config / ".env").exists())
        self.assertTrue((self.slack_config / ".env.example").exists())
        self.assertIn("ConditionPathExists=", self.unit.read_text())


if __name__ == "__main__":
    unittest.main()
