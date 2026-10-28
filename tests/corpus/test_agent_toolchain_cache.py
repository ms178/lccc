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
    def test_untrusted_extract_fails_before_open_without_any_safe_extractor(self):
        spec = importlib.util.spec_from_file_location('agent_cache_restore_test', RESTORE)
        self.assertIsNotNone(spec)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with mock.patch.object(module, 'extractall_takes_filter', return_value=False), \
                mock.patch.object(module, 'confined_extraction_supported', return_value=False), \
                mock.patch.object(module.tarfile, 'open') as archive_open:
            with self.assertRaisesRegex(RuntimeError, 'filter or confined POSIX operations'):
                module.extract_untrusted(Path('not-opened.tar'), Path('not-created'))
        archive_open.assert_not_called()

    def test_confined_extractor_rejects_unsafe_members_before_writing(self):
        import io
        import tarfile
        import tempfile
        spec = importlib.util.spec_from_file_location('agent_cache_confined_test', RESTORE)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        if not module.confined_extraction_supported():
            self.skipTest('descriptor-confined extraction is unavailable')

        def info(name, kind=tarfile.REGTYPE, target=''):
            member = tarfile.TarInfo(name)
            member.type = kind
            member.linkname = target
            return member

        cases = [
            [info('../outside')],
            [info('/absolute')],
            [info('escape', tarfile.SYMTYPE, '../../outside')],
            [info('absolute_link', tarfile.SYMTYPE, '/tmp')],
            [info('root/link', tarfile.SYMTYPE, 'inside'), info('root/link/file')],
            [info('missing', tarfile.LNKTYPE, 'elsewhere')],
            [info('self', tarfile.LNKTYPE, 'self')],
            [info('device', tarfile.CHRTYPE)],
            [info('dup'), info('dup')],
        ]
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            for index, bad_members in enumerate(cases):
                with self.subTest(index=index):
                    archive = base / f'bad-{index}.tar'
                    with tarfile.open(archive, 'w') as out:
                        first = info('would-have-been-written')
                        first.size = 1
                        out.addfile(first, io.BytesIO(b'x'))
                        for member in bad_members:
                            out.addfile(member)
                    prefix = base / f'prefix-{index}'
                    prefix.mkdir()
                    # Exercise the compatibility path even when running on a
                    # newer Python with the native extraction-filter API.
                    with mock.patch.object(module, 'extractall_takes_filter', return_value=False), \
                            mock.patch.object(module.tarfile.TarFile, 'extractall',
                                              side_effect=AssertionError('unfiltered extraction')):
                        with self.assertRaises(module.UnsafeArchiveError):
                            module.extract_untrusted(archive, prefix)
                    self.assertEqual(list(prefix.iterdir()), [])

    def test_confined_extractor_does_not_follow_existing_links(self):
        import io
        import os
        import tarfile
        import tempfile
        spec = importlib.util.spec_from_file_location('agent_cache_existing_link_test', RESTORE)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        if not module.confined_extraction_supported():
            self.skipTest('descriptor-confined extraction is unavailable')
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            outside = base / 'outside'
            outside.mkdir()
            canary = outside / 'canary'
            canary.write_bytes(b'unchanged')
            prefix = base / 'prefix'
            prefix.mkdir()
            os.symlink(outside, prefix / 'parent')
            archive = base / 'bad-parent.tar'
            with tarfile.open(archive, 'w') as out:
                member = tarfile.TarInfo('parent/canary')
                member.size = 7
                out.addfile(member, io.BytesIO(b'corrupt'))
            with self.assertRaises(OSError):
                module.extract_untrusted(archive, prefix)
            self.assertEqual(canary.read_bytes(), b'unchanged')

            (prefix / 'parent').unlink()
            os.symlink(canary, prefix / 'symlink')
            os.link(canary, prefix / 'hardlink')
            archive = base / 'safe.tar'
            with tarfile.open(archive, 'w') as out:
                for name in ('symlink', 'hardlink'):
                    member = tarfile.TarInfo(name)
                    member.size = 3
                    member.mode = 0o6777
                    out.addfile(member, io.BytesIO(b'new'))
                link = tarfile.TarInfo('copied')
                link.type = tarfile.LNKTYPE
                link.linkname = 'hardlink'
                out.addfile(link)
                sym = tarfile.TarInfo('alias')
                sym.type = tarfile.SYMTYPE
                sym.linkname = 'hardlink'
                out.addfile(sym)
                group_exec_only = tarfile.TarInfo('group-exec-only')
                group_exec_only.size = 3
                group_exec_only.mode = 0o010
                out.addfile(group_exec_only, io.BytesIO(b'new'))
            entries = module.extract_untrusted(archive, prefix)
            self.assertTrue(module.warm_members_match(prefix, entries))
            self.assertEqual((prefix / 'group-exec-only').stat().st_mode & 0o111, 0)
            self.assertEqual(canary.read_bytes(), b'unchanged')
            for name in ('symlink', 'hardlink', 'copied', 'alias'):
                self.assertEqual((prefix / name).read_bytes(), b'new')
            self.assertEqual((prefix / 'hardlink').stat().st_mode & 0o7777, 0o755)
            self.assertEqual((prefix / 'hardlink').stat().st_ino, (prefix / 'copied').stat().st_ino)

    def test_atomic_writer_does_not_truncate_fixed_temporary_hardlink(self):
        import os
        import tempfile
        spec = importlib.util.spec_from_file_location('agent_cache_atomic_test', RESTORE)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            canary = base / 'canary'
            canary.write_bytes(b'unchanged')
            target = base / 'wrapper'
            os.link(canary, base / 'wrapper.tmp')
            module.write_atomic(target, b'replacement', 0o755)
            self.assertEqual(canary.read_bytes(), b'unchanged')
            self.assertEqual(target.read_bytes(), b'replacement')
            self.assertEqual(target.stat().st_mode & 0o777, 0o755)

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
                with self.assertRaisesRegex(RuntimeError, 'filter or confined POSIX operations'):
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
