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
            'comiss ucomiss cvtsi2ss cvtsi2ssl cvtsi2ssq cvtss2si cvtss2sil '
            'cvtss2siq cvttss2si cvttss2sil cvttss2siq cmpss cmpps '
            'rcpss rcpps rsqrtss rsqrtps movntps cvtpi2ps cvtps2pi cvttps2pi').split())
# The cmpCCss/cmpCCps pseudo-op spellings of the same SSE1 encoding.
SSE1 |= {f'cmp{cc}{sfx}' for cc in ('eq', 'lt', 'le', 'unord', 'neq', 'nlt', 'nle', 'ord')
         for sfx in ('ss', 'ps')}
FLOAT = ('add_ps', 'sub_ps', 'mul_ps', 'and_ps', 'andnot_ps', 'or_ps', 'xor_ps',
         'add_ss', 'sub_ss', 'mul_ss', 'div_ss', 'min_ss', 'max_ss',
         'sqrt_ss', 'sqrt_ps', 'div_ps', 'max_ps', 'min_ps', 'shuffle_ps', 'setzero_ps',
         'rcp_ss', 'rcp_ps', 'rsqrt_ss', 'rsqrt_ps')
UNARY = ('sqrt_ss', 'sqrt_ps', 'rcp_ss', 'rcp_ps', 'rsqrt_ss', 'rsqrt_ps')
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


#: A compiler that cannot open a *system* header never reached a verdict, so
#: "nonzero exit" is not "rejected". The `-m32` probes below include
#: `<emmintrin.h>`, which pulls the libc headers, and an image without the
#: 32-bit libc development package stops there -- measured: `bits/
#: libc-header-start.h: No such file or directory` from GCC 16.2 and from the
#: distro GCC alike. Treating that as a rejection made this gate demand that
#: lccc reproduce a failure the oracle never made.
HOST_GAP = re.compile(r'fatal error: \S+: No such file or directory')


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
    skipped = 0
    with tempfile.TemporaryDirectory(prefix='legacy-sse-') as directory:
        directory = Path(directory)
        source, assembly = directory / 'probe.c', directory / 'probe.s'

        def probe(text, flags, accepted, feature=None, legal=False, avx1_zero=False, require=None,
                  forbid=None):
            nonlocal count, skipped
            source.write_text(text)
            # Ask the oracle first: without a verdict from it there is nothing
            # to compare, and its own inability to preprocess the probe says
            # nothing about lccc.
            oracle = run([args.gcc, '-O2', *flags, '-S', str(source), '-o', str(assembly)])
            if oracle.returncode != 0 and HOST_GAP.search(oracle.stderr):
                skipped += 1
                print(f'  skip {flags}: the oracle cannot preprocess this probe on this host',
                      flush=True)
                return
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
                if accepted and require and compiler == args.lccc:
                    if not re.search(require, assembly.read_text()):
                        raise RuntimeError(f'{flags}: no {require} in lccc output\n{text}')
                if accepted and forbid and compiler == args.lccc:
                    if re.search(forbid, assembly.read_text()):
                        raise RuntimeError(f'{flags}: {forbid} in lccc output\n{text}')
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
            # Scalar routes that only appear with register homes: a float
            # select lowered through GPR cmov and an __int128 -> float
            # libcall result.
            scalar_routes = [
                'float f(float a, int k, float b) { return k & 1 ? a * b + 0.5f : a - b; }',
                # FP-compare selects are a CMPSS mask plus ANDPS/XORPS bit
                # select (swapped LT/LE for > and >=, +0.0 arm shortcut).
                'float f(float a, float b, float c) { return a < b ? c : a * c; }',
                'float f(float a, float b) { return a >= b ? 0.0f : b - a; }',
                # float -> int is SSE1 CVTTSS2SI, not an SSE2 route.
                'int f(float x, float y) { return (int)(x * y) + (int)y; }',
                # Loop-carried floats across calls: back-edge copies reload
                # spilled F32 phis into XMM homes (was an 8-byte MOVSD) and
                # move GPR-homed bit-cast floats into XMM homes (was MOVQ).
                'unsigned g(float, float);\n'
                'float f(const float *p, int n) {\n'
                '  float a = p[0], b = p[1], c = p[2], d = p[3]; unsigned acc = 0;\n'
                '  for (int i = 0; i < n; i++) {\n'
                '    acc += g(a, d);\n'
                '    if (a < c) b = a - c; else acc ^= g(b, b);\n'
                '    acc += g(c, b);\n'
                '    d = (float)(int)(c * 4.0f) * 0.125f;\n'
                '    if (b < d) c = b - d; else acc ^= g(c, a);\n'
                '  }\n'
                '  return a + b + c + d + (float)acc;\n'
                '}\n',
                'unsigned g(float, float);\n'
                'static unsigned bits(float f) { unsigned u; __builtin_memcpy(&u, &f, 4); return u; }\n'
                'static float fromb(unsigned u) { float f; __builtin_memcpy(&f, &u, 4); return f; }\n'
                'unsigned f(const float *p, int n) {\n'
                '  float a = p[0], b = p[1], c = p[2]; unsigned acc = 0;\n'
                '  for (int i = 0; i < n; i++) {\n'
                '    acc += g(a, b) + (bits(c) >> 3);\n'
                '    b = fromb((bits(c) & 0x807fffffu) | 0x3f000000u);\n'
                '    if (a < b) c = a - b; else acc ^= bits(c);\n'
                '    acc += g(b, c);\n'
                '  }\n'
                '  return acc + bits(a) + bits(b) + bits(c);\n'
                '}\n',
            ]
            if mode == '-m64':
                scalar_routes.append('float f(__int128 x) { return (float)x * 2.0f; }')
            for text in scalar_routes:
                probe(text, [mode, '-mno-sse2', '-msse'], True, legal=True)
            for suffix, ty in (('ps', '__m256'), ('pd', '__m256d'), ('si256', '__m256i')):
                text = f'#include <immintrin.h>\n{ty} f(void) {{ return _mm256_setzero_{suffix}(); }}\n'
                probe(text, [mode, '-mno-avx', '-mno-avx2', '-msse2'], False, '-mavx')
                probe(text, [mode, '-mavx', '-mno-avx2'], True, avx1_zero=True)
            for name in FLOAT:
                operands = '' if name == 'setzero_ps' else ('a' if name in UNARY else ('a,b,0x1b' if name == 'shuffle_ps' else 'a,b'))
                text = f'#include <xmmintrin.h>\n__m128 f(__m128 a,__m128 b) {{ return _mm_{name}({operands}); }}\n'
                probe(text, [mode, '-mno-sse', '-mno-sse2'], False, '-msse')
                for flags in (['-mno-sse2', '-msse'], ['-msse', '-mno-sse2']):
                    probe(text, [mode, *flags], True, legal=True)
            # MOVNTPS is SSE1: the public and raw spellings are granted by
            # -msse alone, lower to the non-temporal store itself (not a
            # plain store) and are refused without SSE.
            # The SSE1 16-byte loads/stores and SFENCE: granted by -msse alone
            # (they were SSE2-gated, refusing every SSE1-only program that
            # touched memory) and refused without SSE, as in GCC.
            for text in (
                '#include <xmmintrin.h>\n__m128 f(const float *p) { return _mm_loadu_ps(p); }\n',
                '#include <xmmintrin.h>\n__m128 f(const float *p) { return _mm_load_ps(p); }\n',
                '#include <xmmintrin.h>\nvoid f(float *p, __m128 a) { _mm_storeu_ps(p, a); }\n',
                '#include <xmmintrin.h>\nvoid f(float *p, __m128 a) { _mm_store_ps(p, a); }\n',
                '#include <xmmintrin.h>\nvoid f(float *d, const float *s) '
                '{ _mm_storeu_ps(d, _mm_mul_ps(_mm_loadu_ps(s), _mm_loadu_ps(s + 4))); }\n',
                '#include <xmmintrin.h>\nvoid f(void) { _mm_sfence(); }\n',
            ):
                probe(text, [mode, '-mno-sse', '-mno-sse2'], False, '-msse')
                for flags in (['-mno-sse2', '-msse'], ['-msse', '-mno-sse2']):
                    probe(text, [mode, *flags], True, legal=True)
            for call in ('_mm_stream_ps(p, a)', '__builtin_ia32_movntps(p, a)'):
                text = f'#include <xmmintrin.h>\nvoid f(float *p, __m128 a) {{ {call}; }}\n'
                probe(text, [mode, '-mno-sse', '-mno-sse2'], False, '-msse')
                for flags in (['-mno-sse2', '-msse'], ['-msse', '-mno-sse2']):
                    probe(text, [mode, *flags], True, legal=True, require=r'\bmovntps\b')
            # _mm_stream_pi / __builtin_ia32_movntq: SSE1-gated as in GCC. The
            # store is MOVNTI from a GPR when SSE2 is usable (one on x86-64,
            # two on i686). An SSE1-only target gets a plain store: MOVNTI is
            # SSE2, and lccc never touches MMX/x87-aliased state (GCC uses
            # MOVNTQ %mm there).
            for text in (
                '#include <xmmintrin.h>\nvoid f(__m64 *p, __m64 a) { _mm_stream_pi(p, a); }\n',
                'void f(unsigned long long *p, unsigned long long v) { __builtin_ia32_movntq(p, v); }\n',
            ):
                probe(text, [mode, '-mno-sse', '-mno-sse2'], False, '-msse')
                for flags in (['-mno-sse2', '-msse'], ['-msse', '-mno-sse2']):
                    probe(text, [mode, *flags], True, legal=True, forbid=r'\bmovnti|%mm\d')
                probe(text, [mode, '-msse2'], True, require=r'\bmovnti', forbid=r'%mm\d')
            # A target("sse2") caller in an SSE1-only TU streams with MOVNTI.
            text = ('#include <xmmintrin.h>\n__attribute__((target("sse2"))) '
                    'void f(__m64 *p, __m64 a) { _mm_stream_pi(p, a); }\n')
            probe(text, [mode, '-mno-sse2', '-msse'], True, require=r'\bmovnti', forbid=r'%mm\d')
            for suffix, ty in (('si128', '__m128i'), ('pd', '__m128d')):
                text = f'#include <emmintrin.h>\n{ty} f(void) {{ return _mm_setzero_{suffix}(); }}\n'
                for flags in (['-mno-sse2', '-msse'], ['-msse', '-mno-sse2']):
                    probe(text, [mode, *flags], False, '-msse2')
                probe(text, [mode, '-msse2'], True)
            # CLFLUSH is SSE2-gated, as in GCC: a Pentium III has SSE1 but no
            # CLFLUSH.
            text = '#include <emmintrin.h>\nvoid f(void *p) { _mm_clflush(p); }\n'
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
    if skipped:
        print(f'PASS: {count} legacy SSE permission / legality compiler legs; {skipped} probe '
              f'mode(s) skipped (this host cannot preprocess them, the oracle included)')
    else:
        print(f'PASS: {count} legacy SSE permission / legality compiler legs; both ABIs, zero skips')


if __name__ == '__main__':
    main()
