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

    def test_binutils_installer_self_test_passes(self):
        result = subprocess.run(
            ['bash', str(ROOT / 'scripts/ensure_binutils_247_binary.sh'), '--self-test'],
            cwd=ROOT, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            timeout=120)
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn('agent cache restore self-test: PASS', result.stdout)

    def test_binutils_installer_without_print_bin_exits_zero(self):
        import os
        import stat
        import tempfile
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            fake_repo = tmp_path / 'repo'
            (fake_repo / 'scripts').mkdir(parents=True)
            (fake_repo / 'engineering/toolchains').mkdir(parents=True)
            (fake_repo / 'engineering/toolchains/agent-cache-lock.json').write_text('{"tag": "fake-tag"}\n')
            # The installer's --verify-only leg sources the real strict version
            # matcher, so the scaffold must carry it at its in-repo path; copying
            # the reviewed helper keeps this test honest about what a banner must
            # satisfy rather than stubbing the check away.
            fake_versions = fake_repo / 'tools/linker'
            fake_versions.mkdir(parents=True)
            (fake_versions / 'oracle_versions.sh').write_text(
                (ROOT / 'tools/linker/oracle_versions.sh').read_text())
            src_script = (ROOT / 'scripts/ensure_binutils_247_binary.sh').read_text()
            dst_script = fake_repo / 'scripts/ensure_binutils_247_binary.sh'
            dst_script.write_text(src_script)
            dst_script.chmod(0o755)
            fake_restore = fake_repo / 'scripts/restore_agent_toolchain_cache.py'
            fake_restore.write_text(
                'import argparse, os, pathlib\n'
                'p = argparse.ArgumentParser()\n'
                'p.add_argument("--tag"); p.add_argument("--prefix"); p.add_argument("--repo"); p.add_argument("--remote"); p.add_argument("--only")\n'
                'a = p.parse_args()\n'
                'b = pathlib.Path(a.prefix) / "bin"\n'
                'b.mkdir(parents=True, exist_ok=True)\n'
                'for t in ("as", "ld", "ld.bfd", "objdump", "readelf"):\n'
                '    f = b / t\n'
                '    f.write_text("#!/bin/sh\\necho GNU " + t + " 2.47\\n")\n'
                '    f.chmod(0o755)\n'
            )
            prefix = tmp_path / 'prefix'
            res_silent = subprocess.run(
                ['bash', str(dst_script), '--prefix', str(prefix)],
                text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=30)
            self.assertEqual(res_silent.returncode, 0, res_silent.stdout)
            self.assertEqual(res_silent.stdout.strip(), f'binutils 2.47 binary oracle OK: {prefix}/bin')

            res_print = subprocess.run(
                ['bash', str(dst_script), '--prefix', str(prefix), '--print-bin'],
                text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=30)
            self.assertEqual(res_print.returncode, 0, res_print.stdout)
            self.assertTrue(res_print.stdout.strip().endswith(f'{prefix}/bin'), res_print.stdout)


if __name__ == '__main__':
    unittest.main()
