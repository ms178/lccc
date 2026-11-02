#!/usr/bin/env python3
"""Packed-product unroll gate: exact overlap semantics and observable ablation."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / 'tests/regression/i32_map_unroll_alias.c'
PRODUCT = ROOT / 'tests/regression/sse2_i32_product.c'


def run(cmd, env):
    p = subprocess.run(cmd, env=env, capture_output=True, timeout=90)
    if p.returncode:
        raise RuntimeError(f'{cmd}: exit {p.returncode}\n{p.stderr.decode(errors="replace")}')
    return p.stdout


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--lccc', default=os.environ.get('CCC', str(ROOT / 'target/fastbuild/lccc')))
    p.add_argument('--gcc', default=os.environ.get('GCC', shutil.which('gcc-16') or 'gcc'))
    p.add_argument('--clang', default=os.environ.get('CLANG', shutil.which('clang-23')))
    p.add_argument('--json', type=Path)
    args = p.parse_args()
    report = {'complete': False, 'source_sha256': hashlib.sha256(SOURCE.read_bytes()).hexdigest(), 'arms': []}
    env = dict(os.environ)
    modes = [('gcc', args.gcc, '-march=x86-64', {})]
    if args.clang:
        modes.append(('clang', args.clang, '-march=x86-64', {}))
    for profile, arch in [('sse2', 'x86-64'), ('sse41', 'x86-64-v2'), ('avx2', 'x86-64-v3')]:
        for rolled in [False, True]:
            modes.append((profile + ('-rolled' if rolled else ''), args.lccc, '-march=' + arch,
                          {'CCC_NO_MAP_I32_UNROLL': '1'} if rolled else {}))
    expected = None
    counts = {}
    try:
        with tempfile.TemporaryDirectory(prefix='i32-map-unroll-') as td:
            for name, cc, arch, extra in modes:
                arm_env = dict(env, **extra)
                if cc == args.lccc:
                    arm_env['CCC_VALIDATE_SSA'] = '1'
                binary = Path(td) / name
                cmd = [cc, '-O2', arch, str(SOURCE), '-o', str(binary)]
                run(cmd, arm_env)
                output = run([str(binary)], arm_env)
                if expected is None:
                    expected = output
                if output != expected:
                    raise RuntimeError(f'{name}: overlap semantics differ from GCC')
                row = {'name': name, 'command': cmd, 'env': extra,
                       'cases': len(output.splitlines()), 'sha256': hashlib.sha256(output).hexdigest()}
                if cc == args.lccc:
                    asm = binary.with_suffix('.s')
                    run([cc, '-O2', '-fwrapv', arch, '-S', str(PRODUCT), '-o', str(asm)], arm_env)
                    match = re.search(r'(?ms)^product_map:\n(.*?)^\.size product_map,', asm.read_text())
                    if not match:
                        raise RuntimeError(f'{name}: missing product_map')
                    body = match[1]
                    # Each SSE2 reconstruction contains two even-lane multiplies;
                    # SSE4.1/AVX2 use one direct multiply per packed operation.
                    count = len(re.findall(r'^\s+(?:v?pmuludq|v?pmulld)\s', body, re.M))
                    row['packed_multiply_count'] = count
                    row['assembly_sha256'] = hashlib.sha256(body.encode()).hexdigest()
                    counts[name] = count
                report['arms'].append(row)
                print(f'{name}: PASS {row["cases"]} overlap/length cases')
            for profile in ['sse2', 'sse41', 'avx2']:
                if not counts[profile + '-rolled'] or counts[profile] != 2 * counts[profile + '-rolled']:
                    raise RuntimeError(f'{profile}: unroll ablation not observable: {counts}')
            report['complete'] = True
    finally:
        if args.json:
            args.json.parent.mkdir(parents=True, exist_ok=True)
            args.json.write_text(json.dumps(report, indent=2) + '\n')


if __name__ == '__main__':
    main()
