#!/usr/bin/env python3
"""Regenerate the GCC-16.2-derived SIMD permission golden table.

Source of truth: the *installed* GCC 16.2.0 intrinsic headers (as delivered
by the `harness-cache-8` tag cache, per MANIFEST.sha256).  For every public
`_mm*` wrapper in `include/lcccsimd.h` we resolve the GCC intrinsic it maps
to, look up which GCC header declares it, and record the AVX-512 feature
bits that header membership implies.  This is exactly the class of evidence
SA-03/SA-04/SA-49 were missing: a mechanical, reproducible sweep instead of
a hand-maintained stem list.

    python3 tests/simd/gen_isa_permissions.py \
        --headers /path/to/gcc-16/lib/gcc/x86_64-pc-linux-gnu/16.2.0/include \
        --output tests/simd/isa_permissions_gcc16.json

The golden file is committed so CI (`check_isa_permissions.py`) can enforce
it without installing GCC headers.  Regenerate only when the generated
header or the pinned GCC oracle changes; the checker fails closed on any
new/removed wrapper.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_HEADER = ROOT / 'include' / 'lcccsimd.h'
DEFAULT_OUT = Path(__file__).with_name('isa_permissions_gcc16.json')

# GCC header -> feature bits that header membership *implies* (a lower bound).
# A byte/word VL form additionally needs BW; that comes from the classifier's
# stem list, so the golden never claims to be the complete feature set.
HEADER_FEATURES: dict[str, list[str]] = {
    'avx512fintrin.h': ['Avx512F'],
    'avx512bwintrin.h': ['Avx512F', 'Avx512Bw'],
    'avx512dqintrin.h': ['Avx512F', 'Avx512Dq'],
    'avx512vlintrin.h': ['Avx512F', 'Avx512Vl'],
    'avx512bitalgintrin.h': ['Avx512F', 'Avx512BitAlg'],
    'avx512vpopcntdqintrin.h': ['Avx512F', 'Avx512Vpopcntdq'],
    'avx512vnniintrin.h': ['Avx512F', 'Avx512Vnni'],
    'avx512vpclmulqdqintrin.h': ['Avx512F', 'Vpclmulqdq'],
    # VL-composed headers: all three bits are implied by the header name.
    'avx512vlbwintrin.h': ['Avx512F', 'Avx512Bw', 'Avx512Vl'],
    'avx512vldqintrin.h': ['Avx512F', 'Avx512Dq', 'Avx512Vl'],
    'avx512vlvnniintrin.h': ['Avx512F', 'Avx512Vnni', 'Avx512Vl'],
    'avx512vlbitalgintrin.h': ['Avx512F', 'Avx512BitAlg', 'Avx512Vl'],
}
# Features the Rust classifier can currently express.  Anything else is
# recorded as `enforce: false` with an explicit reason rather than silently
# dropped — an honest, auditable open set.
MODELLED = {f for features in HEADER_FEATURES.values() for f in features}
# A legacy VEX name that GCC 16.2 also declares inside an AVX-512 header for
# a masked variant.  Header ownership is ambiguous for this spelling, so it
# is recorded but not enforced; `vextractf128` genuinely needs only AVX.
AMBIGUOUS = {'vextractf128'}

WRAPPER_RE = re.compile(
    r'#define\s+(_mm[A-Za-z0-9_]+)\s*\([^)]*\)\s+.*?'
    r'__lccc_simd[A-Za-z0-9_]*?_([a-z][a-z0-9_]*)\s*\(')
INTRINSIC_RE = re.compile(r'\b(_mm(?:512|256|128|)?_[A-Za-z0-9_]+)\s*\(')


def strip_carrier(mnemonic: str) -> str:
    """Strip the generated name's vector-carrier prefix.

    `i_vpshuflw512` -> `vpshuflw512`, `ps_vextractf128` -> `vextractf128`.
    All six carriers are produced by scripts/gen_lcccsimd.py.
    """
    return re.sub(r'^(?:[ifd]|ps|pd|pi)_', '', mnemonic)


def index_headers(directory: Path) -> dict[str, str]:
    owner: dict[str, str] = {}
    headers = sorted(directory.glob('*intrin.h'))
    if not headers:
        raise SystemExit(f'no *intrin.h files under {directory}')
    for path in headers:
        text = path.read_text(encoding='utf-8', errors='replace')
        for name in set(INTRINSIC_RE.findall(text)):
            owner.setdefault(name, path.name)
    return owner


def build(header: Path, gcc_include: Path) -> dict:
    text = header.read_text(encoding='utf-8', errors='replace')
    pairs = WRAPPER_RE.findall(text)
    if not pairs:
        raise SystemExit(f'no generated wrappers found in {header}')
    owner = index_headers(gcc_include)
    rows: dict[str, dict] = {}
    for intrinsic, raw_mnemonic in pairs:
        mnemonic = strip_carrier(raw_mnemonic)
        gcc_header = owner.get(intrinsic, '')
        features = HEADER_FEATURES.get(gcc_header, [])
        model = all(f in MODELLED for f in features)
        ambiguous = mnemonic in AMBIGUOUS
        note = ''
        if ambiguous:
            note = ('legacy VEX spelling also declared in an AVX-512 header '
                    'for a masked variant; header ownership is ambiguous')
        elif not model:
            note = 'feature not modelled by simd_feature_requirements (open)'
        rows[mnemonic] = {
            'intrinsic': intrinsic,
            'gcc_header': gcc_header,
            'required_features': features,
            'enforce': bool(features) and model and not ambiguous,
            'note': note,
        }
    return {
        'schema': 1,
        'source': 'GCC 16.2.0 intrinsic headers (harness-cache-8 tag cache)',
        'gcc_include': str(gcc_include),
        'generated_from': 'include/lcccsimd.h',
        'intrinsic_headers_indexed': len(set(owner.values())),
        'wrapper_pairs': len(pairs),
        'unique_mnemonics': len(rows),
        'rows': dict(sorted(rows.items())),
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--headers', type=Path, required=True)
    parser.add_argument('--generated-header', type=Path, default=DEFAULT_HEADER)
    parser.add_argument('--output', type=Path, default=DEFAULT_OUT)
    args = parser.parse_args(argv)
    doc = build(args.generated_header, args.headers)
    args.output.write_text(json.dumps(doc, indent=2, sort_keys=True) + '\n',
                           encoding='utf-8')
    enforce = sum(r['enforce'] for r in doc['rows'].values())
    print(f'wrote {args.output}: {doc["unique_mnemonics"]} mnemonics, '
          f'{enforce} enforced rows, '
          f'{len(doc["rows"]) - enforce} recorded open/unmapped')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
