#!/usr/bin/env python3
"""Legacy SSE public/raw permissions and SSE1-only instruction legality, both ABIs."""
import argparse
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
SSE1 = set(('movss movaps movups movhps movlps movhlps movlhps unpcklps unpckhps '
            'shufps movmskps addss addps subss subps mulss mulps divss divps '
            'minss minps maxss maxps sqrtss sqrtps xorps andps andnps orps '
            'comiss ucomiss cvtsi2ss cvtsi2ssl cvtsi2ssq').split())
FLOAT = ('add_ps', 'sub_ps', 'mul_ps', 'and_ps', 'andnot_ps', 'or_ps', 'xor_ps',
         'add_ss', 'sub_ss', 'mul_ss', 'div_ss', 'min_ss', 'max_ss',
         'sqrt_ss', 'max_ps', 'min_ps', 'shuffle_ps', 'setzero_ps')
INTEGER = ('add_epi8', 'add_epi16', 'add_epi32', 'add_epi64', 'sub_epi8',
           'sub_epi16', 'sub_epi32', 'sub_epi64', 'and_si128', 'or_si128',
           'xor_si128', 'andnot_si128', 'mullo_epi16', 'mulhi_epi16',
           'madd_epi16', 'adds_epi8', 'adds_epi16', 'adds_epu8', 'adds_epu16',
           'packs_epi16', 'packs_epi32', 'packus_epi16', 'cmpeq_epi8',
           'cmpeq_epi16', 'cmpeq_epi32', 'cmpgt_epi8', 'cmpgt_epi16',
           'cmpgt_epi32', 'avg_epu8', 'avg_epu16', 'min_epu8', 'max_epu8',
           'min_epi16', 'max_epi16', 'sad_epu8', 'mul_epu32', 'mulhi_epu16',
           'subs_epi8', 'subs_epu8', 'subs_epi16', 'subs_epu16',
           'unpacklo_epi8', 'unpackhi_epi8', 'unpacklo_epi16',
           'unpackhi_epi16', 'unpacklo_epi32', 'unpackhi_epi32',
           'unpacklo_epi64', 'unpackhi_epi64', 'sll_epi16', 'srl_epi16')
DOUBLE = ('add_pd', 'sub_pd', 'mul_pd', 'and_pd', 'andnot_pd', 'or_pd', 'xor_pd',
          'add_sd', 'sub_sd', 'mul_sd', 'div_sd', 'min_sd', 'max_sd')


def run(command):
    return subprocess.run(command, capture_output=True, text=True, timeout=90)


def check_legal(text):
    for line in text.splitlines():
        code = line.split('#')[0].strip()
        if not code or code.startswith('.') or code.endswith(':'):
            continue
        if re.search(r'%(?:xmm|ymm|zmm)\d+', code) and code.split()[0] not in SSE1:
            raise RuntimeError('non-SSE1 instruction: ' + code)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lccc', default=os.environ.get('CCC', str(ROOT / 'target/fastbuild/lccc')))
    parser.add_argument('--gcc', default=os.environ.get('GCC', shutil.which('gcc-16') or 'gcc-16'))
    args = parser.parse_args()
    version = run([args.gcc, '-dumpfullversion'])
    if version.returncode or version.stdout.strip() != '16.2.0':
        raise RuntimeError('GCC 16.2.0 is required, not a substituted oracle')
    count = 0
    with tempfile.TemporaryDirectory(prefix='legacy-sse-') as directory:
        directory = Path(directory)
        source, assembly = directory / 'probe.c', directory / 'probe.s'

        def probe(text, flags, accepted, feature=None, legal=False, avx1_zero=False):
            nonlocal count
            source.write_text(text)
            for compiler in (args.lccc, args.gcc):
                assembly.unlink(missing_ok=True)
                result = run([compiler, '-O2', *flags, '-S', str(source), '-o', str(assembly)])
                if (result.returncode == 0) != accepted:
                    raise RuntimeError(f'{compiler} {flags}: expected accept={accepted}\n{text}\n{result.stderr}')
                if compiler == args.lccc and not accepted:
                    if feature not in result.stderr or (assembly.exists() and assembly.stat().st_size):
                        raise RuntimeError('missing feature diagnostic or assembly emitted after rejection: ' + result.stderr)
                if accepted and legal:
                    check_legal(assembly.read_text())
                if accepted and avx1_zero:
                    # VEX.128 PXOR clears the entire YMM register on AVX1;
                    # VEX.256 integer PXOR would silently require AVX2.
                    if re.search(r'\bvpxor\b[^\n]*%ymm', assembly.read_text()):
                        raise RuntimeError('AVX1 zero constructor emitted AVX2 VPXOR')
                count += 1

        for mode in ('-m32', '-m64'):
            for text in (
                'unsigned f(float x) { union { float f; unsigned u; } a={.f=x}; return a.u; }',
                'float f(unsigned x) { union { float f; unsigned u; } a={.u=x}; return a.f; }',
            ):
                probe(text, [mode, '-mno-sse2', '-msse'], True, legal=True)
            for suffix, ty in (('ps', '__m256'), ('pd', '__m256d'), ('si256', '__m256i')):
                text = f'#include <immintrin.h>\n{ty} f(void) {{ return _mm256_setzero_{suffix}(); }}\n'
                probe(text, [mode, '-mno-avx', '-mno-avx2', '-msse2'], False, '-mavx')
                probe(text, [mode, '-mavx', '-mno-avx2'], True, avx1_zero=True)
            for name in FLOAT:
                operands = '' if name == 'setzero_ps' else ('a' if name == 'sqrt_ss' else ('a,b,0x1b' if name == 'shuffle_ps' else 'a,b'))
                text = f'#include <xmmintrin.h>\n__m128 f(__m128 a,__m128 b) {{ return _mm_{name}({operands}); }}\n'
                probe(text, [mode, '-mno-sse', '-mno-sse2'], False, '-msse')
                for flags in (['-mno-sse2', '-msse'], ['-msse', '-mno-sse2']):
                    probe(text, [mode, *flags], True, legal=True)
            for suffix, ty in (('si128', '__m128i'), ('pd', '__m128d')):
                text = f'#include <emmintrin.h>\n{ty} f(void) {{ return _mm_setzero_{suffix}(); }}\n'
                for flags in (['-mno-sse2', '-msse'], ['-msse', '-mno-sse2']):
                    probe(text, [mode, *flags], False, '-msse2')
                probe(text, [mode, '-msse2'], True)
            for name in INTEGER + DOUBLE:
                ty = '__m128i' if name in INTEGER else '__m128d'
                text = f'#include <emmintrin.h>\n{ty} f({ty} a,{ty} b) {{ return _mm_{name}(a,b); }}\n'
                for flags in (['-mno-sse2', '-msse'], ['-msse', '-mno-sse2']):
                    probe(text, [mode, *flags], False, '-msse2')
                probe(text, [mode, '-msse2'], True)
            text = ('typedef float v4sf __attribute__((vector_size(16)));\n'
                    'v4sf f(v4sf a,v4sf b) { return __builtin_ia32_addps(a,b); }\n')
            probe(text, [mode, '-mno-sse', '-mno-sse2'], False, '-msse')
            probe(text, [mode, '-mno-sse2', '-msse'], True, legal=True)
            # A target-scoped grant is local, not a global permission leak.
            text = ('#include <emmintrin.h>\n'
                    '__attribute__((target("sse2"))) __m128i f(__m128i a,__m128i b) '
                    '{ return _mm_add_epi32(a,b); }\n')
            probe(text, [mode, '-mno-sse2', '-msse'], True)
            text += '__m128i g(__m128i a,__m128i b) { return _mm_add_epi32(a,b); }\n'
            probe(text, [mode, '-mno-sse2', '-msse'], False, '-msse2')
    print(f'PASS: {count} legacy SSE permission / legality compiler legs; both ABIs, zero skips')


if __name__ == '__main__':
    main()
