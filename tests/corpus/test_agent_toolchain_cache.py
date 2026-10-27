"""The GitHub-only agent cache must round-trip from a synthetic orphan tag."""
import subprocess
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
RESTORE = ROOT / 'scripts/restore_agent_toolchain_cache.py'


class AgentToolchainCache(unittest.TestCase):
    def test_restore_self_test_reassembles_and_extracts_a_tag(self):
        result = subprocess.run(
            [sys.executable, str(RESTORE), '--self-test'],
            cwd=ROOT, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            timeout=120)
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn('agent cache restore self-test: PASS', result.stdout)


if __name__ == '__main__':
    unittest.main()

    def test_binutils_installer_self_test_passes(self):
        result = subprocess.run(
            ['bash', str(ROOT / 'scripts/ensure_binutils_247_binary.sh'), '--self-test'],
            cwd=ROOT, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            timeout=120)
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn('agent cache restore self-test: PASS', result.stdout)
