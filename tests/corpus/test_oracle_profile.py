"""Canary admission is strict and cannot stand in for performance evidence."""
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
import godbolt
import oracle_profile as profile

ASSEMBLY = ['vpaddd %ymm1, %ymm0, %ymm0', 'vfmadd213ps %xmm2, %xmm1, %xmm0',
            'vcvtph2ps %xmm0, %ymm0', 'andnq %rsi, %rdi, %rax',
            'pdepq %rsi, %rdi, %rax', 'lzcntq %rdi, %rax',
            'popcntq %rdi, %rax', 'vpminsd %xmm1, %xmm0, %xmm0', 'crc32l %esi, %eax']


class ProfileTests(unittest.TestCase):
    def verify(self, lines=ASSEMBLY, error=None):
        response = {'code': 0, 'asm': [{'text': s} for s in lines]}
        with patch.object(godbolt, 'compiler_metadata', return_value={'id': 'cicc2021100'}), \
                patch.object(godbolt, '_request_json', return_value=response, side_effect=error):
            return profile.verify(['icc'], '-O2 -march=haswell -fwrapv')

    def test_admission_records_actual_target_and_feature_evidence(self):
        report = self.verify()
        self.assertTrue(report['complete'])
        row = report['compilers']['icc']
        self.assertEqual(row['effective_flags'], '-O2 -xCORE-AVX2 -fwrapv')
        self.assertEqual(set(row['features']), set(profile.FEATURES))
        self.assertTrue(report['compile_only'])
        self.assertEqual(len(row['assembly_sha256']), 64)
        self.assertIn('NOT runtime', report['metric'])

    def test_contraction_off_requires_separate_mul_and_add(self):
        for instructions, expected in ((['vmulsd %xmm1, %xmm0, %xmm0',
                                         'vaddsd %xmm2, %xmm0, %xmm0'], True),
                                       (['vfmadd213sd %xmm2, %xmm1, %xmm0'], False),
                                       ([], False)):
            responses = [{'code': 0, 'asm': [{'text': x} for x in text]}
                         for text in (ASSEMBLY, instructions)]
            with self.subTest(instructions=instructions), \
                    patch.object(godbolt, 'compiler_metadata', return_value={'id': 'cicc2021100'}), \
                    patch.object(godbolt, '_request_json', side_effect=responses):
                result = profile.verify(['icc'], '-O2 -march=haswell -ffp-contract=off')
            self.assertEqual(result['complete'], expected)
            self.assertEqual(result['compilers']['icc']['contraction_off']['ok'], expected)
    def test_classic_icc_value_safe_dialect_is_explicit(self):
        flags = '-O2 -march=haswell -fno-fast-math -ffp-contract=off -DN=17'
        self.assertEqual(godbolt.compiler_flags('icc', flags),
            '-O2 -xCORE-AVX2 -fp-model precise -no-fma -DN=17')
        self.assertEqual(godbolt.compiler_flags('icx', flags), flags)
        self.assertEqual(godbolt.compiler_flags('gcc16.2', flags), flags)

    def test_each_missing_feature_fails_admission(self):
        for n in range(len(ASSEMBLY)):
            with self.subTest(n=n):
                report = self.verify(ASSEMBLY[:n] + ASSEMBLY[n+1:])
                self.assertFalse(report['complete'])
                self.assertFalse(report['compilers']['icc']['ok'])

    def test_transport_failure_is_not_a_skip_or_vacuous_pass(self):
        report = self.verify(error=godbolt.GodboltError('HTTP 503'))
        self.assertFalse(report['complete'])
        self.assertIn('HTTP 503', report['compilers']['icc']['error'])

    def test_common_target_and_nonempty_compiler_set_are_required(self):
        for names, flags in (([], '-O2 -march=haswell'), (['icc'], '-O2 -march=x86-64-v3')):
            with self.subTest(names=names, flags=flags), self.assertRaises(ValueError):
                profile.verify(names, flags)

    def test_ignored_target_is_rejected_even_with_all_instruction_text(self):
        response = {'code': 0, 'asm': [{'text': s} for s in ASSEMBLY],
                    'stderr': "warning: option '-xCORE-AVX2' not supported"}
        with patch.object(godbolt, 'compiler_metadata', return_value={'id': 'cicc2021100'}), \
                patch.object(godbolt, '_request_json', return_value=response):
            report = profile.verify(['icc'], '-O2 -march=haswell -fwrapv')
        self.assertFalse(report['complete'])
        self.assertIn('not honored', report['compilers']['icc']['error'])


if __name__ == '__main__':
    unittest.main()
