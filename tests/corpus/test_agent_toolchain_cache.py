"""The GitHub-only agent cache must round-trip from a synthetic orphan tag."""
import importlib.util
from pathlib import Path
import subprocess
import sys
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
RESTORE = ROOT / 'scripts/restore_agent_toolchain_cache.py'
BINUTILS = ROOT / 'scripts/ensure_binutils_247_binary.sh'


class AgentToolchainCache(unittest.TestCase):
    def test_untrusted_extract_fails_before_open_without_filter_support(self):
        spec = importlib.util.spec_from_file_location('agent_cache_restore_test', RESTORE)
        self.assertIsNotNone(spec)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with mock.patch.object(module, 'extractall_takes_filter', return_value=False), \
                mock.patch.object(module.tarfile, 'open') as archive_open:
            with self.assertRaisesRegex(RuntimeError, 'filter-capable Python'):
                module.extract_untrusted(Path('not-opened.tar'), Path('not-created'))
        archive_open.assert_not_called()

    def test_restore_self_test_reassembles_and_extracts_a_tag(self):
        result = subprocess.run(
            [sys.executable, str(RESTORE), '--self-test'],
            cwd=ROOT, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            timeout=120)
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn('agent cache restore self-test: PASS', result.stdout)

    def test_binutils_installer_self_test_passes(self):
        result = subprocess.run(
            ['bash', str(BINUTILS), '--self-test'],
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


    def test_escaping_link_chain_is_rejected(self):
        """Members that are each legal can still form an escaping chain."""
        import os
        import tempfile
        spec = importlib.util.spec_from_file_location('agent_cache_chain_test', RESTORE)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            prefix = base / 'prefix'
            (prefix / 'other').mkdir(parents=True)
            (base / 'outside').mkdir()
            os.symlink('root/link', prefix / 'bin')
            (prefix / 'root').mkdir()
            os.symlink('inside/../../outside', prefix / 'root/link')
            os.symlink('../other', prefix / 'root/inside')
            # Every individual target is legal; only the closed chain escapes.
            with self.assertRaisesRegex(RuntimeError, 'escapes the cache prefix'):
                module.reject_escaping_links(prefix)

    def test_contained_links_are_accepted(self):
        import os
        import tempfile
        spec = importlib.util.spec_from_file_location('agent_cache_contained_test', RESTORE)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as tmp:
            prefix = Path(tmp) / 'prefix'
            (prefix / 'tool/bin').mkdir(parents=True)
            os.symlink('gcc-16', prefix / 'tool/bin/cc')
            os.symlink('../tool/bin/gcc-16', prefix / 'tool/bin/alias')
            module.reject_escaping_links(prefix)  # must not raise

    def test_filter_probe_precedes_part_reassembly(self):
        """An unsupported host must fail before materialising 500 MiB."""
        import subprocess as sp
        import tempfile
        spec = importlib.util.spec_from_file_location('agent_cache_probe_test', RESTORE)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as tmp:
            repo = Path(tmp) / 'repo'
            repo.mkdir()
            sp.run(['git', 'init', '-q', '-b', 'main', str(repo)], check=True)
            sp.run(['git', '-C', str(repo), '-c', 'user.name=t', '-c', 'user.email=t@e.invalid',
                    'commit', '-q', '--allow-empty', '-m', 'x'], check=True)
            sp.run(['git', '-C', str(repo), 'tag', 'synthetic'], check=True)
            with mock.patch.object(module, 'extraction_supported', return_value=False), \
                    mock.patch.object(module, 'materialise') as materialise, \
                    mock.patch.object(module, 'read_blob') as read_blob:
                with self.assertRaisesRegex(RuntimeError, 'filter-capable Python'):
                    module.restore(repo, '', 'synthetic', Path(tmp) / 'prefix', None,
                                   skip_lock=True, only=None, no_shims=False,
                                   verify_only=False, force=False)
            materialise.assert_not_called()
            read_blob.assert_not_called()

    def test_missing_lock_file_is_an_error(self):
        import tempfile
        spec = importlib.util.spec_from_file_location('agent_cache_lock_test', RESTORE)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(RuntimeError, 'does not exist'):
                module.restore(Path(tmp), '', 'synthetic', Path(tmp) / 'prefix',
                               Path(tmp) / 'absent-lock.json', skip_lock=False, only=None,
                               no_shims=False, verify_only=False, force=False)


if __name__ == '__main__':
    unittest.main()
