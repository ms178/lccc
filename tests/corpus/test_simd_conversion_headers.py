"""Independent signature and harness controls for explicit SIMD conversion."""
import importlib.util
from pathlib import Path
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]


class ConversionHeaders(unittest.TestCase):
    def test_packed_signatures(self):
        header = (ROOT / 'include/lcccsimd.h').read_text()
        # Explicit ISA contracts, independent of the generator's inference.
        contracts = {
            'cvtps_epi32': ('s', 'i', 'same'),
            'cvttps_epi32': ('s', 'i', 'same'),
            'cvtepi32_ps': ('i', 's', 'same'),
            'cvtps_pd': ('s', 'd', 'widen'),
            'cvtepi32_pd': ('i', 'd', 'widen'),
            'cvtpd_ps': ('d', 's', 'narrow'),
            'cvtpd_epi32': ('d', 'i', 'narrow'),
            'cvttpd_epi32': ('d', 'i', 'narrow'),
        }
        for width in (128, 256, 512):
            for name, (src, dst, shape) in contracts.items():
                sw = max(128, width // 2) if shape == 'widen' else width
                dw = max(128, width // 2) if shape == 'narrow' else width
                suffix = {'s': '', 'i': 'i', 'd': 'd'}
                prefix = '_mm_' if width == 128 else f'_mm{width}_'
                signature = f'__m{dw}{suffix[dst]} {prefix}{name}(__m{sw}{suffix[src]} __a)'
                with self.subTest(width=width, name=name):
                    self.assertEqual(header.count(signature), 1)

    def test_scalar_and_cast_signatures(self):
        header = (ROOT / 'include/lcccsimd.h').read_text()
        for fragment in (
            '__m128d _mm_cvtss_sd(__m128d __a, __m128 __b)',
            '__m128 _mm_cvtsd_ss(__m128 __a, __m128d __b)',
            '__m256i _mm512_castsi512_si256(__m512i __a)',
            'int _mm256_testz_ps(__m256 __a, __m256 __b)',
            'int _mm256_movemask_pd(__m256d __a)',
            '__lccc_simd128_ps_cvttss2si128(0, __a)',
            '__lccc_simd128_ps_cvttsd2si128(0, __a)',
            '__lccc_simd128_ps_cvtsi2sd64_128(long, __m128d __a, long long __i)',
        ):
            self.assertIn(fragment, header)


class IntrinsicHarness(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location('intrinsic_runner', ROOT / 'tests/intrinsics/run_intrin_tests.py')
        cls.runner = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.runner)

    def run_case(self, results, missing):
        with tempfile.TemporaryDirectory() as tmp:
            src = Path(tmp) / 'contract.c'
            src.write_text('// FLAGS: -mavx512f\nint main(void) { return 0; }\n')
            args = SimpleNamespace(lccc_flags='-O2', gcc_flags='-O2', verbose=False)
            with patch.object(self.runner, 'run', side_effect=results) as run, patch.object(self.runner, 'missing_isa', return_value=missing):
                status = self.runner.run_cases(args, [src], Path(tmp))
                return status, run.call_count

    @staticmethod
    def result(code=0, stdout='ok', stderr=''):
        return subprocess.CompletedProcess([], code, stdout, stderr)

    def test_missing_isa_still_assembles(self):
        self.assertEqual(self.run_case([self.result(), self.result()], ['avx512f']), (0, 2))

    def test_missing_isa_does_not_hide_bad_codegen(self):
        self.assertEqual(self.run_case([self.result(1), self.result()], ['avx512f']), (1, 2))

    def test_oracle_compile_failure_is_not_a_skip(self):
        self.assertEqual(self.run_case([self.result(), self.result(1)], []), (1, 2))

    def test_equal_crashes_are_not_agreement(self):
        self.assertEqual(self.run_case([self.result(), self.result(), self.result(-11), self.result(-11)], []), (1, 4))

    def test_success_requires_matching_output(self):
        self.assertEqual(self.run_case([self.result(), self.result(), self.result(), self.result()], []), (0, 4))
        self.assertEqual(self.run_case([self.result(), self.result(), self.result(stdout='a'), self.result(stdout='b')], []), (1, 4))


if __name__ == '__main__':
    unittest.main()
