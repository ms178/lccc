"""Source-level contracts for the rootless cross-toolchain provisioner.

The execution proof belongs to the cross suites.  This small regression test
keeps the host discovery repair from silently reverting on minimal agent/root
PATHs, where /sbin is commonly absent even though ldconfig is installed.
"""
from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts/ensure_cross_toolchains.sh"


class CrossProvisionerTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.text = SCRIPT.read_text(encoding="utf-8")

    def test_shell_syntax(self):
        subprocess.run(["bash", "-n", str(SCRIPT)], check=True)

    def test_ldconfig_has_standard_sbin_fallbacks(self):
        self.assertIn("ldconfig_bin=$(command -v ldconfig 2>/dev/null || true)", self.text)
        self.assertIn("for candidate in /sbin/ldconfig /usr/sbin/ldconfig; do", self.text)
        self.assertIn('"$ldconfig_bin" || loader_cache_failed=1', self.text)
        self.assertIn('sudo -n "$ldconfig_bin" || loader_cache_failed=1', self.text)
        self.assertNotIn("sudo -n ldconfig", self.text)

    def test_archive_name_uses_apt_filename_not_digest(self):
        # Execute the production function, not a Python reimplementation.  The
        # real apt format ends in SHA256:<digest>; treating $NF as the archive
        # name silently disabled both cache hits and download accounting.
        start = self.text.index("archive_name() {")
        function = self.text[start:self.text.index("\n}", start) + 2]
        probe = function + r'''
apt-get() {
    [ "$1" = download ] && [ "$2" = --print-uris ] && [ "$3" = example-package ]
    printf "'https://invalid.example/example_1.0_amd64.deb' example_1.0_amd64.deb 123 SHA256:deadbeef\n"
}
apt_opts=()
result=$(archive_name example-package)
[ "$result" = example_1.0_amd64.deb ]
'''
        result = subprocess.run(["bash", "-c", probe], text=True, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
