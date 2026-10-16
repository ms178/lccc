"""Compile-only Haswell feature canary; not correctness or runtime evidence.

Direct vendor intrinsics force representative instructions for each feature.
A failed compilation, ignored option or missing instruction fails the profile.
CPU flags name a common Haswell target (ICC dialect CORE-AVX2); this is not
silently relabeled as an identical x86-64-v3 option set or newer CPU tuning.
"""
from __future__ import annotations

import hashlib
import re
import shlex
import tempfile
from pathlib import Path

import godbolt

SOURCE = '''#include <immintrin.h>
__m256i avx2(__m256i a, __m256i b) { return _mm256_add_epi32(a,b); }
__m128 fma(__m128 a, __m128 b, __m128 c) { return _mm_fmadd_ps(a,b,c); }
__m256 f16c(__m128i a) { return _mm256_cvtph_ps(a); }
unsigned long long bmi1(unsigned long long a,unsigned long long b) { return _andn_u64(a,b); }
unsigned long long bmi2(unsigned long long a,unsigned long long b) { return _pdep_u64(a,b); }
unsigned long long lzcnt(unsigned long long a) { return _lzcnt_u64(a); }
unsigned long long popcnt(unsigned long long a) { return _mm_popcnt_u64(a); }
__m128i sse41(__m128i a,__m128i b) { return _mm_min_epi32(a,b); }
unsigned sse42(unsigned a,unsigned b) { return _mm_crc32_u32(a,b); }
'''
CONTRACTION_SOURCE = "double no_contract(double a, double b, double c) { return a*b+c; }\n"
FEATURES = {
    'AVX2': r'\bvpaddd\b[^\n]*%ymm', 'FMA': r'\bvfmadd(?:132|213|231)ps\b',
    'F16C': r'\bvcvtph2ps\b', 'BMI1': r'\bandnq?\b', 'BMI2': r'\bpdepq?\b',
    'LZCNT': r'\blzcntq?\b', 'POPCNT': r'\bpopcntq?\b',
    'SSE4.1': r'\bv?pminsd\b', 'SSE4.2': r'\bcrc32[lq]?\b',
}


def verify(oracles, flags):
    if not oracles or '-march=haswell' not in shlex.split(flags):
        raise ValueError('Haswell profile verification requires oracles and an explicit common Haswell target')
    report = {'target': 'Haswell (ICC classic CORE-AVX2 dialect)', 'compile_only': True,
              'metric': 'feature/option admission, NOT runtime or cross-vendor semantic equivalence',
              'source_sha256': hashlib.sha256(SOURCE.encode()).hexdigest(),
              'source': SOURCE, 'requested_flags': flags, 'compilers': {}, 'complete': False}
    for name in oracles:
        local = name in godbolt.LOCAL_ORACLES
        cid = name if local else godbolt.resolve_compiler(name)
        row = {'id': cid, 'requested_flags': flags, 'local': local}
        report['compilers'][name] = row
        try:
            if local:
                row['metadata'] = godbolt.local_oracle_metadata(name)
                row['effective_flags'] = flags

                def compile_text(source_text, _name=name, _flags=flags):
                    # Local oracles compile a file on this host; the CE survey
                    # canary must never see them. Source goes to a private dir.
                    with tempfile.TemporaryDirectory(prefix='lccc-profile-') as td:
                        src = Path(td) / 'profile.c'
                        src.write_text(source_text)
                        return godbolt.compile_local_oracle(_name, src, _flags), []
            else:
                row['metadata'] = godbolt.compiler_metadata(cid)
                row['effective_flags'] = godbolt.compiler_flags(cid, flags)

                def compile_text(source_text, _cid=cid, _flags=flags):
                    result = godbolt.compile_on_godbolt(_cid, source_text, _flags,
                                                        raise_on_error=True)
                    if result is None:
                        raise godbolt.GodboltError('missing response')
                    return godbolt.assembly_lines(result), result.get('stderr', [])
            lines, stderr = compile_text(SOURCE)
            text = '\n'.join(lines)
            row['features'] = {key: bool(re.search(pattern, text)) for key, pattern in FEATURES.items()}
            row['assembly_sha256'] = hashlib.sha256(text.encode()).hexdigest()
            row['assembly'] = lines
            row['stderr'] = stderr
            row['ok'] = all(row['features'].values())
            if '-ffp-contract=off' in shlex.split(flags):
                asm, probe_stderr = compile_text(CONTRACTION_SOURCE)
                body = '\n'.join(asm)
                separate = (bool(re.search(r'\bv?mulsd\b', body))
                            and bool(re.search(r'\bv?addsd\b', body))
                            and not re.search(r'\bv?f(?:madd|msub|nmadd|nmsub)', body))
                row['contraction_off'] = {
                    'source': CONTRACTION_SOURCE, 'assembly': asm,
                    'assembly_sha256': hashlib.sha256(body.encode()).hexdigest(),
                    'ok': bool(separate), 'stderr': probe_stderr}
                row['ok'] = row['ok'] and bool(separate)
            if not row['ok']:
                row['error'] = 'missing representative ISA instruction(s) or contraction-off witness'
        except (godbolt.GodboltError, ValueError) as error:
            row['ok'] = False
            row['error'] = str(error)
    report['complete'] = all(row['ok'] for row in report['compilers'].values())
    return report
