"""Revision comparisons must retain truthful names, flags and binary provenance."""
from argparse import Namespace
from pathlib import Path
import sys
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
from tests.benchmark import run_benchmarks as bench


class NamedCandidates(unittest.TestCase):
    def arguments(self, *candidates):
        return Namespace(compilers='lccc,gcc', candidate=list(candidates),
                         lccc='/bin/final', gcc='/bin/gcc', clang='', icx='',
                         ccc='', lev='', opt='-O3', cflag=['-fwrapv', '-march=x86-64-v3'])

    def discover(self, args, *, missing=(), include='/gcc/include'):
        def resolve(name):
            return name if name and name not in missing else None
        with mock.patch.object(bench, 'compiler_path', side_effect=resolve), \
             mock.patch.object(bench, 'detect_gcc_include', return_value=include):
            return bench.discover_compilers(args)

    def test_labels_are_not_original_compiler_or_vendor_aliases(self):
        compilers, unavailable = self.discover(self.arguments('prior=/bin/prior', 'pr835=/bin/competitor'))
        self.assertEqual([item.key for item in compilers], ['lccc', 'gcc', 'prior', 'pr835'])
        self.assertFalse(unavailable)
        self.assertEqual(compilers[-1].executable, '/bin/competitor')
        self.assertEqual(compilers[-1].label, 'LCCC candidate: pr835')

    def test_every_candidate_preserves_the_common_target_and_semantics(self):
        compilers, _ = self.discover(self.arguments('prior=/bin/prior'))
        self.assertEqual(compilers[0].flags, compilers[-1].flags)
        self.assertEqual(compilers[-1].flags,
                         ('-I/gcc/include', '-O3', '-fwrapv', '-march=x86-64-v3'))

    def test_missing_candidate_is_an_unavailable_arm_not_a_silent_skip(self):
        compilers, unavailable = self.discover(self.arguments('prior=/bin/prior'), missing=['/bin/prior'])
        self.assertEqual([item.key for item in compilers], ['lccc', 'gcc'])
        self.assertEqual(unavailable, {'prior': 'not found: /bin/prior'})

    def test_candidates_require_the_matching_builtin_header_provider(self):
        _, unavailable = self.discover(self.arguments('prior=/bin/prior'), include=None)
        self.assertIn('prior', unavailable)
        self.assertIn('builtin include', unavailable['prior'])

    def test_reserved_duplicate_empty_and_path_labels_are_rejected(self):
        for entries in [('gcc=/evil',), ('lccc=/evil',), ('prior=',), ('prior',),
                        ('../escape=/bin/cc',), ('UPPER=/bin/cc',), ('=cc',),
                        ('prior=/a', 'prior=/b')]:
            with self.subTest(entries=entries), self.assertRaises(ValueError):
                self.discover(self.arguments(*entries))

    def test_equals_and_spaces_in_executable_path_are_not_shell_syntax(self):
        compilers, _ = self.discover(self.arguments('prior=/path with spaces/cc=one'))
        self.assertEqual(compilers[-1].executable, '/path with spaces/cc=one')

    def test_repeated_requested_keys_do_not_duplicate_samples(self):
        args = self.arguments('prior=/bin/prior')
        args.compilers = 'gcc,gcc,lccc'
        compilers, _ = self.discover(args)
        self.assertEqual([item.key for item in compilers], ['gcc', 'lccc', 'prior'])

    def test_legacy_programmatic_callers_need_not_supply_candidates(self):
        args = self.arguments()
        del args.candidate
        compilers, _ = self.discover(args)
        self.assertEqual([item.key for item in compilers], ['lccc', 'gcc'])

    def test_registry_keeps_distinct_macro_sized_shapes(self):
        registry = {item.name: item for item in bench.BENCHMARKS}
        self.assertEqual(len(registry), 45)
        for left, right in [('linux_find_bit', 'linux_find_bit_scaled'),
                            ('lz4_compress', 'lz4_match_extend')]:
            self.assertEqual(bench.source_for(registry[left]), bench.source_for(registry[right]))
            self.assertNotEqual(registry[left].extra_flags, registry[right].extra_flags)


if __name__ == '__main__':
    unittest.main()
