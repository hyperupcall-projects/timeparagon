"""Exercise installer lifecycle in an isolated home layout with stubbed tools."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


class InstallTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="timeparagon-install-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / "repo"
        packaging = self.repo / "packaging"
        packaging.mkdir(parents=True)
        for source in Path(__file__).resolve().parent.iterdir():
            if source.suffix in (".sh", ".in"):
                shutil.copy2(source, packaging / source.name)
        fixtures = {
            "target/debug/build/cef-dll-sys-fixture/out/cef_linux_x86_64/libcef.so": "chromium-fixture",
            "target/debug/build/cef-dll-sys-fixture/out/cef_linux_x86_64/locales/en-US.pak": "locale-fixture",
            "target/debug/timeparagon": "#!/bin/sh\nexit 0\n",
            "target/debug/examples/sync_database": "#!/bin/sh\nexit 0\n",
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
        (self.repo / "target/debug/timeparagon").chmod(0o755)
        (self.repo / "target/debug/examples/sync_database").chmod(0o755)
        stubs = self.root / "tools"
        stubs.mkdir()
        for name in ("node", "npm", "systemctl", "kwriteconfig6", "qdbus6",
                     "update-desktop-database", "gtk-update-icon-cache",
                     "pkill", "pgrep"):
            path = stubs / name
            path.write_text(
                '#!/bin/sh\n'
                f'printf "%s\\n" "{name} $*" >> "$INSTALL_TEST_LOG"\n'
                # No process matches, so nothing outside the test is touched.
                'case " $* " in *" is-active "*) exit 1;; esac\n'
                f'case "{name}" in pkill|pgrep) exit 1;; esac\nexit 0\n'
            )
            path.chmod(0o755)
        # Exercise systemd quoting and sed substitutions in installation paths.
        self.data = self.root / 'data with spaces % & " |'
        self.config = self.root / "config"
        self.prefix = self.root / "prefix"
        self.state = self.root / "state"
        self.cache = self.root / "cache"
        self.home = self.root / "home"
        self.log = self.root / "commands.log"
        # Every location the scripts may touch lives under the scratch root.
        self.env = {
            **os.environ, "PATH": f"{stubs}:{os.environ['PATH']}", "HOME": str(self.home),
            "XDG_DATA_HOME": str(self.data), "XDG_CONFIG_HOME": str(self.config),
            "XDG_STATE_HOME": str(self.state), "XDG_CACHE_HOME": str(self.cache),
            "INSTALL_TEST_LOG": str(self.log),
        }
        self.slack_config = self.config / "timeparagon/slack"
        self.unit = self.config / "systemd/user/timeparagon-slack.service"

    def run_script(self, name, *args):
        return subprocess.run(
            ["sh", str(self.repo / "packaging" / name), "--prefix", str(self.prefix), *args],
            env=self.env, check=True, capture_output=True, text=True,
        )

    def test_install_upgrade_and_uninstall(self):
        self.run_script("install.sh", "--no-build")
        self.assertEqual((self.prefix / "lib/timeparagon/cef/libcef.so").read_text(), "chromium-fixture")
        self.assertTrue((self.prefix / "lib/timeparagon/cef/locales/en-US.pak").exists())
        self.assertTrue((self.data / "timeparagon/slack/src/app.ts").exists())
        self.assertIn("npm ci --omit=dev", self.log.read_text())
        self.assertTrue((self.prefix / "bin/timeparagon-database-sync").exists())
        sync_unit = self.config / "systemd/user/timeparagon-database-sync.service"
        self.assertIn("--watch", sync_unit.read_text())
        self.assertNotIn("@SYNC_BINARY@", sync_unit.read_text())
        self.assertIn("systemctl --user enable timeparagon-database-sync.service", self.log.read_text())
        self.assertIn("systemctl --user enable timeparagon-slack.service", self.log.read_text())
        self.assertIn("systemctl --user restart timeparagon-slack.service", self.log.read_text())
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
        self.assertFalse((self.prefix / "lib/timeparagon/cef").exists())
        self.assertFalse(self.unit.exists())
        self.assertFalse(sync_unit.exists())
        self.assertFalse((self.prefix / "bin/timeparagon-database-sync").exists())
        self.assertFalse((self.data / "timeparagon/slack").exists())
        self.assertEqual(tokens.read_text(), "newer-pair")
        self.assertEqual(installations.read_text(), "newer-workspaces")
        self.run_script("uninstall.sh", "--purge")
        self.assertFalse(self.slack_config.exists())

    def test_upgrade_from_timeslay(self):
        """An install over TimeSlay moves its data and removes what it installed."""
        old_slack = self.config / "timeslay/slack"
        old_slack.mkdir(parents=True)
        (old_slack / ".env").write_text(
            f"SLACK_APP_TOKEN_FILE={old_slack}/.slack-app-tokens.json\n# TIMESLAY_BIN=/x/timeslay\n")
        (old_slack / ".slack-app-tokens.json").write_text("old-pair")
        old_data = self.data / "timeslay"
        old_data.mkdir(parents=True)
        (old_data / "timeslay.sqlite3").write_text("database")
        (old_data / "timeslay-webextension").write_text("#!/bin/sh\n")
        (self.state / "timeslay").mkdir(parents=True)
        (self.state / "timeslay/secrets.json").write_text("secrets")
        legacy = [
            self.prefix / "bin/timeslay", self.prefix / "bin/time-slay",
            self.prefix / "bin/timeslay-database-sync", self.prefix / "lib/timeslay/cef/libcef.so",
            self.data / "applications/timeslay.desktop",
            self.data / "icons/hicolor/512x512/apps/timeslay.png",
            self.config / "systemd/user/timeslay-slack.service",
            self.home / ".mozilla/native-messaging-hosts/com.timeslay.webextension.json",
        ]
        for path in legacy:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("old")
        webextension = self.repo / "webextension/install.sh"
        webextension.parent.mkdir()
        webextension.write_text('#!/bin/sh\nprintf "webextension %s\\n" "$TIMEPARAGON_BIN" >> "$INSTALL_TEST_LOG"\n')
        webextension.chmod(0o755)
        (self.config / "kwinrulesrc").write_text("[General]\nOrder=TimeSlay Sidebar\n\n[TimeSlay Sidebar]\n")

        self.run_script("install.sh", "--no-build")

        for path in legacy:
            self.assertFalse(path.exists(), path)
        self.assertFalse((self.prefix / "lib/timeslay").exists())
        self.assertFalse((self.config / "timeslay").exists())
        self.assertFalse(old_data.exists())
        self.assertEqual((self.data / "timeparagon/timeslay.sqlite3").read_text(), "database")
        self.assertFalse((self.data / "timeparagon/timeslay-webextension").exists())
        self.assertEqual((self.state / "timeparagon/secrets.json").read_text(), "secrets")
        self.assertEqual((self.slack_config / ".slack-app-tokens.json").read_text(), "old-pair")
        env_file = (self.slack_config / ".env").read_text()
        self.assertIn(f"{self.slack_config}/.slack-app-tokens.json", env_file)
        self.assertIn("TIMEPARAGON_BIN", env_file)
        self.assertNotIn("timeslay", env_file.lower())
        commands = self.log.read_text()
        self.assertIn("systemctl --user disable --now timeslay.service", commands)
        self.assertIn("pkill -x timeslay|time-slay", commands)
        self.assertNotIn("[TimeSlay Sidebar]", (self.config / "kwinrulesrc").read_text())
        self.assertIn(f"webextension {self.prefix / 'bin/timeparagon'}", commands)

    def test_kwin_rule_is_rewritten_cleanly(self):
        """Stale keys in our rule group go away; other rules are untouched."""
        rules = self.config / "kwinrulesrc"
        rules.parent.mkdir(parents=True)
        rules.write_text("[General]\nOrder=Other,TimeParagon Sidebar\n\n"
                         "[Other]\ntitle=keep\n\n"
                         "[TimeParagon Sidebar]\ndeleteRule=true\nstale=1\n\n"
                         "[Tail]\ntitle=keep too\n")
        self.run_script("install.sh", "--no-build")
        text = rules.read_text()
        self.assertNotIn("[TimeParagon Sidebar]", text)
        self.assertNotIn("stale", text)
        self.assertIn("[Other]\ntitle=keep", text)
        self.assertIn("[Tail]\ntitle=keep too", text)
        rules.write_text("[General]\nOrder=Other,TimeParagon Sidebar\n\n[TimeParagon Sidebar]\nstale=1\n\n[Other]\nx=1\n")
        self.run_script("uninstall.sh")
        text = rules.read_text()
        self.assertNotIn("[TimeParagon Sidebar]", text)
        self.assertIn("[Other]\nx=1", text)

    def test_existing_data_is_never_overwritten(self):
        (self.config / "timeslay").mkdir(parents=True)
        (self.config / "timeslay/keep").write_text("old")
        (self.config / "timeparagon").mkdir(parents=True)
        result = self.run_script("install.sh", "--no-build")
        self.assertEqual((self.config / "timeslay/keep").read_text(), "old")
        self.assertIn("Left ", result.stdout)

    def test_no_enable(self):
        self.run_script("install.sh", "--no-build", "--no-enable")
        self.assertTrue(self.unit.exists())
        binary = self.prefix / "bin/timeparagon"
        desktop = (self.data / "applications/timeparagon.desktop").read_text()
        service = (self.config / "systemd/user/timeparagon.service").read_text()
        shortcuts = (self.config / "systemd/user/timeparagon-shortcuts.service").read_text()
        self.assertIn(f"Exec={binary} sidebar\n", desktop)
        self.assertIn(f"ExecStart={binary} sidebar\n", service)
        self.assertIn(f"ExecStart={binary} daemon\n", shortcuts)
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
