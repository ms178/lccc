#!/usr/bin/env python3
"""Fail-closed ratchet for x86 SIMD feature permissions.

`isa_permissions_gcc16.json` is generated from the pinned GCC 16.2.0 headers
by `gen_isa_permissions.py`.  This checker enforces, without needing a
compiler or those headers installed:

  1. the generated wrapper set has not changed (a new `_mm*` wrapper must be
     added to the golden table on purpose, not silently inherit no features);
  2. for every enforced golden row, `simd_feature_requirements` in
     src/ir/lowering/expr_builtins_lcccsimd.rs returns ALL features that GCC
     header membership implies;
  3. the two digit-less EVEX load/store mnemonics are classified as EVEX
     (SA-03) — they have no wrapper `#define` and would otherwise be missed.

The classifier is read from the Rust source rather than duplicated: stem
lists, `matches!` eVEX names, BW_EXACT and DQ_EXACT are parsed, so a fix in
Rust is what makes this checker green.  An unknown feature or a silently
removed rule is an ERROR, never a pass.

    python3 tests/simd/check_isa_permissions.py
    python3 tests/simd/check_isa_permissions.py --self-test
    python3 tests/simd/check_isa_permissions.py --json /tmp/receipts/isa.json
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
if str(HERE) not in sys.path:
    sys.path.insert(0, str(HERE))
try:
    from probe_utils import write_json
except ImportError as exc:  # pragma: no cover
    raise SystemExit(f'check_isa_permissions: probe_utils unavailable: {exc}') from exc

GOLDEN = HERE / 'isa_permissions_gcc16.json'
GENERATED = ROOT / 'include' / 'lcccsimd.h'
RUST = ROOT / 'src' / 'ir' / 'lowering' / 'expr_builtins_lcccsimd.rs'
WRAPPER_RE = re.compile(
    r'#define\s+(_mm[A-Za-z0-9_]+)\s*\([^)]*\)\s+.*?'
    r'__lccc_simd[A-Za-z0-9_]*?_([a-z][a-z0-9_]*)\s*\(')
DIGITLESS_EVEX = ('vmovdqu64_load', 'vmovdqu64_store')


def strip_carrier(name: str) -> str:
    return re.sub(r'^(?:[ifd]|ps|pd|pi)_', '', name)


def current_wrappers(text: str) -> dict[str, str]:
    # Last definition wins, exactly like gen_isa_permissions.py: several
    # `_mm*` spellings (cmp/cmpeq) map to one mnemonic, and both sides must
    # resolve the same representative intrinsic or the diff is noise.
    out: dict[str, str] = {}
    for intrinsic, raw in WRAPPER_RE.findall(text):
        out[strip_carrier(raw)] = intrinsic
    return out


def slice_between(text: str, start: str, end: str) -> str:
    a = text.index(start)
    b = text.index(end, a)
    return text[a:b]


def parse_classifier(text: str) -> dict:
    """Extract the rule sets from simd_feature_requirements."""
    body = slice_between(text, 'fn simd_feature_requirements',
                         'fn push_feature')
    bw_block = slice_between(body, 'let bw = [', '];')
    bw = set(re.findall(r'^\s*"([a-z0-9_]+)",', bw_block, re.M))
    ev = slice_between(body, 'matches!(', ');')
    evex_names = set(re.findall(r'"([a-z0-9_]+)"', ev))
    bw_exact = set(re.findall(
        r'"([a-z0-9_]+)"',
        slice_between(body, 'const BW_EXACT', '];')))
    dq_exact = set(re.findall(
        r'"([a-z0-9_]+)"',
        slice_between(body, 'const DQ_EXACT', '];')))
    return {'bw': bw, 'evex': evex_names, 'bw_exact': bw_exact,
            'dq_exact': dq_exact}


def simulate(rules: dict, mnemonic: str) -> set[str]:
    """Mirror of the Rust classifier.  Kept deliberately small and literal."""
    is512 = mnemonic.endswith('512') or mnemonic.endswith('_512')
    mask_cmp_low = (mnemonic.startswith('vpcmp') and
                    (mnemonic.endswith('128') or mnemonic.endswith('256')))
    evex = (is512 or mask_cmp_low or mnemonic.startswith('vpternlog')
            or mnemonic.startswith('vmovdqu8_mask')
            or mnemonic in rules['evex'])
    feats: set[str] = set()
    if evex:
        feats.add('Avx512F')
    if (any(mnemonic.startswith(s) for s in rules['bw'])
            or mnemonic in rules['bw_exact']):
        feats.add('Avx512Bw')
    if mnemonic in rules['dq_exact']:
        feats.add('Avx512Dq')
    if mnemonic.startswith('vpopcntb') or mnemonic.startswith('vpopcntw'):
        feats.add('Avx512BitAlg')
    if mnemonic.startswith('vpopcntd') or mnemonic.startswith('vpopcntq'):
        feats.add('Avx512Vpopcntdq')
    if mnemonic.startswith('vpdpbusd'):
        feats.add('Avx512Vnni')
    if (mnemonic.startswith('vpternlog') or mnemonic.startswith('vpcmp')
            or mnemonic.startswith('vmovdqu8_mask')
            or mnemonic in ('vpshufb_maskz128', 'vpshufb_mask128')) and not is512:
        feats.add('Avx512Vl')
    return feats if evex else set()


def check(golden: dict, generated_text: str, rust_text: str) -> tuple[int, dict]:
    rules = parse_classifier(rust_text)
    wrappers = current_wrappers(generated_text)
    rows = golden['rows']
    problems: list[str] = []

    if golden.get('schema') != 1:
        problems.append(f'unknown golden schema {golden.get("schema")!r}')
    new = sorted(set(wrappers) - set(rows))
    gone = sorted(set(rows) - set(wrappers))
    if new:
        problems.append(f'{len(new)} wrapper(s) missing from golden: {new}')
    if gone:
        problems.append(f'{len(gone)} golden row(s) no longer generated: {gone}')
    for name, intrinsic in wrappers.items():
        if name in rows and rows[name]['intrinsic'] != intrinsic:
            problems.append(f'{name}: intrinsic changed to {intrinsic!r}')

    enforced = 0
    for name, row in sorted(rows.items()):
        if not row['enforce']:
            continue
        enforced += 1
        need = set(row['required_features'])
        got = simulate(rules, name)
        missing = sorted(need - got)
        if missing:
            problems.append(f'{name}: {row["gcc_header"]} requires '
                            f'{missing}, classifier gave {sorted(got)}')

    for name in DIGITLESS_EVEX:
        if name not in rules['evex']:
            problems.append(f'{name}: not classified as EVEX (SA-03 regression)')
        if simulate(rules, name) != {'Avx512F'}:
            problems.append(f'{name}: expected exactly [Avx512F], '
                            f'got {sorted(simulate(rules, name))}')

    summary = {
        'schema': 1,
        'checker': 'check_isa_permissions',
        'golden_source': golden.get('source'),
        'golden_mnemonics': len(rows),
        'enforced_rows': enforced,
        'open_rows': len(rows) - enforced,
        'wrappers': len(wrappers),
        'bw_stems': len(rules['bw']),
        'evex_names': len(rules['evex']),
        'bw_exact': len(rules['bw_exact']),
        'dq_exact': len(rules['dq_exact']),
        'problems': problems,
    }
    return len(problems), summary


def self_test() -> int:
    good = {
        'schema': 1, 'source': 'test',
        'rows': {
            'vpshuflw512': {'intrinsic': '_mm512_shufflelo_epi16',
                            'gcc_header': 'avx512bwintrin.h',
                            'required_features': ['Avx512F', 'Avx512Bw'],
                            'enforce': True, 'note': ''},
            'vpshufb512': {'intrinsic': '_mm512_shuffle_epi8',
                           'gcc_header': 'avx512bwintrin.h',
                           'required_features': ['Avx512F', 'Avx512Bw'],
                           'enforce': True, 'note': ''},
        },
    }
    gen = ('#define _mm512_shufflelo_epi16(a,b) '
           '__CCC(__lccc_simd512_i_vpshuflw512(b,a))\n'
           '#define _mm512_shuffle_epi8(a,b) '
           '__CCC(__lccc_simd512_i_vpshufb512(b,a))\n')
    rust = ('fn simd_feature_requirements(m: &str) {\n'
            'let bw = [\n    "vpshufb",\n];\n'
            'matches!(m, "x" | "vmovdqu64_load" | "vmovdqu64_store");\n'
            'const BW_EXACT: &[&str] = &["vpshuflw512",];\n'
            'const DQ_EXACT: &[&str] = &["vdummy",];\n'
            '}\nfn push_feature() {}')
    checks: list[tuple[str, bool]] = []
    n, _ = check(good, gen, rust)
    checks.append(('clean table passes', n == 0))
    broken = json.loads(json.dumps(good))
    broken['rows']['vpshuflw512']['required_features'].append('Avx512Dq')
    checks.append(('missing feature detected', check(broken, gen, rust)[0] > 0))
    checks.append(('new wrapper detected',
                   check(good, gen + '#define _mm_new_x() '
                         '__CCC(__lccc_simd_i_vnew())\n', rust)[0] > 0))
    no_evex = rust.replace(' | "vmovdqu64_load" | "vmovdqu64_store"', '')
    checks.append(('digit-less EVEX regression detected',
                   check(good, gen, no_evex)[0] > 0))
    checks.append(('carrier stripping',
                   strip_carrier('ps_vextractf128') == 'vextractf128'))
    failed = [label for label, ok in checks if not ok]
    for label, ok in checks:
        print(f'{"PASS" if ok else "FAIL"}  {label}')
    print(f'self-test: {len(checks) - len(failed)}/{len(checks)} checks passed')
    return 1 if failed else 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--json', type=Path)
    parser.add_argument('--self-test', action='store_true')
    args = parser.parse_args(argv)
    if args.self_test:
        return self_test()
    for path in (GOLDEN, GENERATED, RUST):
        if not path.is_file():
            print(f'ERROR missing input: {path}')
            return 1
    try:
        golden = json.loads(GOLDEN.read_text(encoding='utf-8'))
        n, summary = check(golden, GENERATED.read_text(encoding='utf-8'),
                           RUST.read_text(encoding='utf-8'))
    except (ValueError, KeyError, json.JSONDecodeError) as exc:
        print(f'ERROR cannot parse inputs: {exc}')
        return 1
    if args.json:
        write_json(args.json, summary)
    print(f'ISA permission ratchet: {summary["wrappers"]} wrappers, '
          f'{summary["enforced_rows"]} enforced rows, '
          f'{summary["open_rows"]} recorded open, {n} problems')
    for problem in summary['problems'][:20]:
        print(f'  {problem}')
    return 1 if n else 0


if __name__ == '__main__':
    raise SystemExit(main())
