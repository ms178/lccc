"""Behavioural contracts for the rootless cross-toolchain provisioner.

The execution proof belongs to the cross suites.  These tests keep the two
host-discovery repairs from silently reverting:

* `archive_name` must read the *filename* out of `apt-get --print-uris`, whose
  real format ends in `SHA256:<digest>` -- treating the last field as the name
  disabled every cache hit;
* `find_ldconfig` must find the standard absolute binary that a minimal
  agent/root `PATH` omits.

Both functions are extracted from the script and executed against stubs, so a
behavioural regression fails here instead of only in a host-dependent lane.  A
source-text assertion would pass on a rewrite that no longer works.
"""
from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "ensure_cross_toolchains.sh"


def extract_function(name: str) -> str:
    """Return the shell source of ``name`` from the provisioner."""
    text = SCRIPT.read_text(encoding="utf-8")
    start = text.index(f"{name}() {{")
    end = text.index("\n}\n", start) + 3
    return text[start:end]


class CrossProvisionerTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.text = SCRIPT.read_text(encoding="utf-8")

    def test_shell_syntax(self):
        subprocess.run(["bash", "-n", str(SCRIPT)], check=True)

    def test_archive_name_uses_apt_filename_not_digest(self):
        # Execute the production function, not a Python reimplementation.  The
        # real apt format ends in SHA256:<digest>; treating $NF as the archive
        # name silently disabled both cache hits and download accounting.
        probe = extract_function("archive_name") + r'''
apt-get() {
    printf "'https://invalid.example/example_1.0_amd64.deb' example_1.0_amd64.deb 123 SHA256:deadbeef\n"
}
apt_opts=()
result=$(archive_name example-package)
[ "$result" = example_1.0_amd64.deb ] || { echo "got: $result" >&2; exit 1; }
'''
        result = subprocess.run(["bash", "-c", probe], text=True, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_cross_generation_uses_system_gcc_not_path_oracle(self):
        import tempfile
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            system = root / "system-gcc"
            system.write_text("#!/bin/sh\necho 13.3.0\n")
            system.chmod(0o755)
            oracle = root / "gcc"
            oracle.write_text("#!/bin/sh\necho 16.2.0\n")
            oracle.chmod(0o755)
            probe = extract_function("system_gcc_major") + f'''
LCCC_CROSS_SYSTEM_GCC="{system}"
PATH="{root}:$PATH"
result=$(system_gcc_major)
[ "$result" = 13 ] || {{ echo "got: [$result]" >&2; exit 1; }}
'''
            result = subprocess.run(["bash", "-c", probe], text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_find_ldconfig_finds_absolute_binary_outside_path(self):
        import tempfile
        with tempfile.TemporaryDirectory() as tmp:
            sbin = Path(tmp) / "sbin"
            sbin.mkdir()
            fake = sbin / "ldconfig"
            fake.write_text("#!/bin/sh\nexit 0\n")
            fake.chmod(0o755)
            probe = extract_function("find_ldconfig") + f'''
CROSS_SBIN_DIRS=("{sbin}" "/nonexistent-sbin")
PATH=/nonexistent-bin
found=$(find_ldconfig)
[ "$found" = "{fake}" ] || {{ echo "got: [$found]" >&2; exit 1; }}
'''
            result = subprocess.run(["bash", "-c", probe], text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_find_ldconfig_is_empty_when_absent(self):
        probe = extract_function("find_ldconfig") + r'''
CROSS_SBIN_DIRS=("/nonexistent-a" "/nonexistent-b")
PATH=/nonexistent-bin
found=$(find_ldconfig)
[ -z "$found" ] || { echo "got: [$found]" >&2; exit 1; }
'''
        result = subprocess.run(["bash", "-c", probe], text=True, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_find_ldconfig_prefers_path_lookup(self):
        import tempfile
        with tempfile.TemporaryDirectory() as tmp:
            bindir = Path(tmp) / "bin"
            bindir.mkdir()
            fake = bindir / "ldconfig"
            fake.write_text("#!/bin/sh\nexit 0\n")
            fake.chmod(0o755)
            probe = extract_function("find_ldconfig") + f'''
CROSS_SBIN_DIRS=("/nonexistent-a")
PATH="{bindir}"
found=$(find_ldconfig)
[ "$found" = "{fake}" ] || {{ echo "got: [$found]" >&2; exit 1; }}
'''
            result = subprocess.run(["bash", "-c", probe], text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_missing_ldconfig_fails_the_step_rather_than_skipping_it(self):
        # A host with no ldconfig at all must be an error: the old code skipped
        # the step and let a later `as --version` probe report something else.
        start = self.text.index("ldconfig_bin=$(find_ldconfig)")
        block = self.text[start:self.text.index("\n# Versioned compiler packages", start)]
        self.assertIn("loader_cache_failed=1", block.split("else", 1)[1])
        self.assertNotIn("sudo -n ldconfig", self.text)


if __name__ == "__main__":
    unittest.main()
