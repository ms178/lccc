"""Offline archive-layout and fail-closed tests; no downloaded tools execute."""
import io
import json
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'scripts'))
import restore_llvm_oracle_slice as llvm


class LLVMOracleSliceTests(unittest.TestCase):
    def test_both_sanitizer_resource_layouts_are_selected(self):
        for path in ('lib/clang/23/lib/linux/libclang_rt.asan-x86_64.a',
                     'lib/clang/23/lib/linux/libclang_rt.ubsan_standalone-x86_64.so',
                     'lib/clang/23/lib/x86_64-unknown-linux-gnu/libclang_rt.asan_static.a',
                     'lib/clang/23/lib/x86_64-unknown-linux-gnu/libclang_rt.asan.a',
                     'lib/clang/23/lib/x86_64-unknown-linux-gnu/libclang_rt.ubsan_standalone.a'):
            self.assertEqual(llvm.selected_path(llvm.TOP + '/' + path), path)
        for path in ('lib/clang/23/lib/linux/libclang_rt.asan-aarch64.a',
                     'lib/clang/23/lib/aarch64-unknown-linux-gnu/libclang_rt.asan.a',
                     'lib/libLLVM.a', 'bin/opt', 'bin/clang-format'):
            self.assertIsNone(llvm.selected_path(llvm.TOP + '/' + path))

    def test_only_the_pinned_archive_root_is_accepted(self):
        self.assertIsNone(llvm.selected_path('other/bin/clang-23'))
        with self.assertRaises(ValueError):
            llvm.selected_path(llvm.TOP + '/lib/clang/23/include/../../escape')

    @staticmethod
    def archive(entries):
        data = io.BytesIO()
        with tarfile.open(fileobj=data, mode='w') as tar:
            for name, kind in entries:
                member = tarfile.TarInfo(llvm.TOP + '/' + name)
                if kind == 'file':
                    member.size = 3
                    tar.addfile(member, io.BytesIO(b'abc'))
                else:
                    member.type = tarfile.SYMTYPE
                    member.linkname = 'clang-23'
                    tar.addfile(member)
        data.seek(0)
        return data

    def test_only_regular_selected_files_are_materialized(self):
        data = self.archive([('bin/clang-23', 'file'), ('bin/clang', 'link'),
                             ('bin/opt', 'file'), ('lib/clang/23/include/stddef.h', 'file')])
        with tempfile.TemporaryDirectory() as td, tarfile.open(fileobj=data) as tar:
            root = Path(td)
            paths = llvm.extract_members(tar, root)
            self.assertEqual(paths, ['bin/clang-23', 'lib/clang/23/include/stddef.h'])
            self.assertFalse((root / 'bin/opt').exists())
            self.assertFalse((root / 'bin/clang').is_symlink())

    def test_header_links_and_duplicate_files_are_refused(self):
        for entries, exception in [([('lib/clang/23/include/a.h', 'link')], ValueError),
                                   ([('bin/clang-23', 'file')] * 2, FileExistsError)]:
            with self.subTest(entries=entries), tempfile.TemporaryDirectory() as td:
                with tarfile.open(fileobj=self.archive(entries)) as tar:
                    with self.assertRaises(exception):
                        llvm.extract_members(tar, Path(td))

    def test_wrong_archive_never_reaches_extraction_or_execution(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            archive = root / 'bad.zst'
            archive.write_bytes(b'wrong')
            with patch.object(llvm.shutil, 'which', return_value='/unused/zstd'), \
                 patch.object(llvm.shutil, 'disk_usage', return_value=SimpleNamespace(free=1 << 40)), \
                 patch.object(llvm, 'probe') as probe, \
                 patch.object(llvm.subprocess, 'Popen') as decoder:
                with self.assertRaisesRegex(ValueError, 'size/SHA-256'):
                    llvm.install(root / 'prefix', archive)
                probe.assert_not_called()
                decoder.assert_not_called()
            self.assertFalse((root / 'prefix').exists())

    def test_foreign_or_incomplete_prefix_never_executes_a_tool(self):
        with tempfile.TemporaryDirectory() as td:
            prefix = Path(td)
            sentinel = prefix / 'keep'
            sentinel.write_text('untouched')
            with patch.object(llvm, 'probe') as probe:
                with self.assertRaises(FileNotFoundError):
                    llvm.install(prefix)
                (prefix / llvm.RECEIPT).write_text(json.dumps({
                    'version': llvm.VERSION, 'archive_sha256': llvm.SHA256,
                    'files': {}, 'aliases': {}}))
                with self.assertRaisesRegex(ValueError, 'incomplete'):
                    llvm.install(prefix)
                probe.assert_not_called()
            self.assertEqual(sentinel.read_text(), 'untouched')


if __name__ == '__main__':
    unittest.main()
