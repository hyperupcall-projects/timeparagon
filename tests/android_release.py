"""Exercise the release publication gates without building Rust or using real keys.

Run with: python3 tests/android_release.py
Actual APK/signature validation is also performed by build-android.sh on builds.
"""

import hashlib
import base64
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


class AndroidReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="android-release-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        scripts = self.root / "scripts"
        scripts.mkdir()
        self.script = scripts / "build-android.sh"
        shutil.copyfile(Path(__file__).resolve().parents[1] / "scripts/build-android.sh", self.script)
        config = self.root / "dioxus/Dioxus.toml"
        config.parent.mkdir()
        self.config_text = '[application]\nname = "TimeSlay"\n'
        config.write_text(self.config_text)
        (self.root / "Cargo.toml").write_text('version = "0.14.0"\n')
        (self.root / "dioxus/android/kotlin").mkdir(parents=True)
        (self.root / "dioxus/android/res").mkdir(parents=True)
        self.apk = self.root / "target/dx/TimeSlay/release/android/app/app/build/outputs/apk/release/app-release.apk"
        self.apk.parent.mkdir(parents=True)
        self.apk.write_bytes(b"fixture apk")
        self.output = self.root / "output"
        self.bin = self.root / "bin"
        self.bin.mkdir()
        sdk = self.root / "sdk"
        (sdk / "ndk/test").mkdir(parents=True)
        build_tools = sdk / "build-tools/34.0.0"
        build_tools.mkdir(parents=True)
        java = self.root / "java"
        (java / "bin").mkdir(parents=True)
        keystore = self.root / "fixture.jks"
        keystore.write_bytes(b"test-only keystore")
        self.env = dict(os.environ, PATH=str(self.bin) + os.pathsep + os.environ["PATH"],
                        JAVA_HOME=str(java), ANDROID_HOME=str(sdk),
                        ANDROID_NDK_HOME=str(sdk / "ndk/test"),
                        TIMESLAY_KEYSTORE_BASE64=base64.b64encode(keystore.read_bytes()).decode(),
                        TIMESLAY_KEYSTORE_PASSWORD="synthetic-password",
                        TIMESLAY_KEYSTORE_KEYALIAS="fixture")
        self.tool(self.bin / "dx", "pass")
        self.tool(self.bin / "rustup", "print('aarch64-linux-android')")
        self.tool(java / "bin/java", "print('openjdk version \"21.0.1\"')")
        self.tool(java / "bin/keytool", "from pathlib import Path\nPath(sys.argv[sys.argv.index('-file')+1]).write_bytes(b'fixture certificate')")
        self.tool(build_tools / "aapt2", """
if os.environ.get('INSPECTION_FAIL'): sys.exit(1)
print('package: name=dev.timeslay.day')
if os.environ.get('DEBUGGABLE'): print('application-debuggable')
""")
        self.tool(build_tools / "apksigner", """
if os.environ.get('SIGNATURE_FAIL'): sys.exit(1)
dn = 'Android Debug' if os.environ.get('DEBUG_SIGNER') else 'Release'
digest = os.environ.get('SIGNER_DIGEST', '')
prefixes = ['V2 Signer: ', 'V3.0 Signer: '] if os.environ.get('MULTI_SCHEME') else \
           ['V2 Signer: '] if os.environ.get('NEW_APKSIGNER') else ['Signer #1 ']
if os.environ.get('EXTRA_SIGNER'):
    prefixes.append('Signer #2 ')
for prefix in prefixes:
    signer_digest = '0' * 64 if prefix == 'Signer #2 ' else digest
    print(prefix + 'certificate DN: CN=' + dn)
    print(prefix + 'certificate SHA-256 digest: ' + signer_digest)
""")
        self.env["SIGNER_DIGEST"] = hashlib.sha256(b"fixture certificate").hexdigest()

    def tool(self, path, code):
        path.write_text(f"#!{sys.executable}\nimport os, sys\n{code}\n")
        path.chmod(0o700)

    def build(self, accepted=False, **environment):
        result = subprocess.run(["sh", str(self.script), "--release", "--output", str(self.output)],
                                env=dict(self.env, **environment), capture_output=True, text=True)
        self.assertEqual(result.returncode == 0, accepted, result.stdout + result.stderr)
        self.assertEqual((self.root / "dioxus/Dioxus.toml").read_text(), self.config_text)
        self.assertEqual((self.output / "timeslay-arm64-v8a-release.apk").exists(), accepted)

    def test_missing_signing_key_is_rejected(self):
        self.build(TIMESLAY_KEYSTORE_BASE64="")

    def test_debuggable_apk_is_rejected(self):
        self.build(DEBUGGABLE="1")

    def test_failed_manifest_inspection_is_rejected(self):
        self.build(INSPECTION_FAIL="1")

    def test_invalid_signature_is_rejected(self):
        self.build(SIGNATURE_FAIL="1")

    def test_debug_certificate_is_rejected(self):
        self.build(DEBUG_SIGNER="1")

    def test_unexpected_signing_key_is_rejected(self):
        self.build(SIGNER_DIGEST="0" * 64)

    def test_expected_release_signer_is_accepted(self):
        self.build(accepted=True)

    def test_new_apksigner_output_format_is_accepted(self):
        # build-tools 37.0.0 labels signers "V2 Signer:" rather than "Signer #1".
        self.build(accepted=True, NEW_APKSIGNER="1")

    def test_new_apksigner_format_unexpected_key_is_rejected(self):
        self.build(NEW_APKSIGNER="1", SIGNER_DIGEST="0" * 64)

    def test_same_signer_for_multiple_schemes_is_accepted(self):
        self.build(accepted=True, MULTI_SCHEME="1")

    def test_multiple_different_signers_are_rejected(self):
        self.build(EXTRA_SIGNER="1")


if __name__ == "__main__":
    unittest.main()
