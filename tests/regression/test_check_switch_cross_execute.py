#!/usr/bin/env python3
"""Linker discovery contracts; real target executions remain in the runner."""
import unittest
from unittest.mock import patch

from check_switch_cross_execute import CONTROL_PASSES, FIXTURES, main, select_linker


class LinkerDiscoveryTests(unittest.TestCase):
    def test_memory_fixtures_run_with_their_controls_in_the_cross_matrix(self):
        for fixture, control in [('narrow_big_endian_cross', 'byte_load'),
                                 ('join_store_forward_cross', 'join_store_forward'),
                                 ('conditional_store_readonly_cross', 'vectorize'),
                                 ('strided_reduction_cross', 'vectorize')]:
            self.assertIn(fixture, FIXTURES)
            self.assertEqual(CONTROL_PASSES[fixture], control)

    @patch('check_switch_cross_execute.subprocess.check_output', side_effect=RuntimeError('probe'))
    @patch('sys.argv', ['check_switch_cross_execute.py'])
    @patch.dict('os.environ', {'CLANG': '/cached/clang-23.1.4'}, clear=True)
    def test_honors_provisioned_clang_environment(self, output):
        with self.assertRaisesRegex(RuntimeError, 'probe'):
            main()
        output.assert_called_once_with(['/cached/clang-23.1.4', '-dumpversion'], text=True)

    @patch('check_switch_cross_execute.subprocess.check_output', side_effect=RuntimeError('probe'))
    @patch('sys.argv', ['check_switch_cross_execute.py', '--clang', '/explicit/clang'])
    @patch.dict('os.environ', {'CLANG': '/cached/clang-23.1.4'}, clear=True)
    def test_explicit_clang_overrides_environment(self, output):
        with self.assertRaisesRegex(RuntimeError, 'probe'):
            main()
        output.assert_called_once_with(['/explicit/clang', '-dumpversion'], text=True)

    def test_explicit_gnu(self):
        self.assertEqual(select_linker('aarch64', '/tool/ld'), ['/tool/ld'])

    def test_explicit_rust(self):
        self.assertEqual(select_linker('riscv64', '/tool/rust-lld'),
                         ['/tool/rust-lld', '-flavor', 'gnu'])

    @patch('check_switch_cross_execute.shutil.which', return_value='/tool/ld')
    def test_prefers_target_gnu(self, which):
        self.assertEqual(select_linker('aarch64'), ['/tool/ld'])
        which.assert_called_once_with('aarch64-linux-gnu-ld')

    @patch('check_switch_cross_execute.shutil.which', side_effect=[None, '/tool/ld.lld'])
    def test_system_lld(self, which):
        self.assertEqual(select_linker('riscv64'), ['/tool/ld.lld'])

    @patch('check_switch_cross_execute.Path.is_file', return_value=True)
    @patch('check_switch_cross_execute.subprocess.check_output', return_value='/rust/lib/rustlib/host/lib\n')
    @patch('check_switch_cross_execute.shutil.which', side_effect=[None, None, '/rust/bin/rustc'])
    def test_bundled_rust_lld(self, which, output, exists):
        self.assertEqual(select_linker('riscv64'),
                         ['/rust/lib/rustlib/host/bin/rust-lld', '-flavor', 'gnu'])

    @patch('check_switch_cross_execute.shutil.which', return_value=None)
    def test_missing_tools_fail_not_skip(self, which):
        with self.assertRaisesRegex(FileNotFoundError, 'No GNU/LLD linker for aarch64'):
            select_linker('aarch64')

    @patch('check_switch_cross_execute.Path.is_file', return_value=False)
    @patch('check_switch_cross_execute.subprocess.check_output', return_value='/rust/lib/rustlib/host/lib\n')
    @patch('check_switch_cross_execute.shutil.which', side_effect=[None, None, '/rust/bin/rustc'])
    def test_missing_bundled_lld_fails(self, which, output, exists):
        with self.assertRaises(FileNotFoundError):
            select_linker('riscv64')


if __name__ == '__main__':
    unittest.main()
