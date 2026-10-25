#!/usr/bin/env python3
"""Strict F32/F64 map unroll: guard pages, aliasing, bitwise outputs and ablation."""
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
SOURCE = ROOT / 'tests/regression/fp_map_unroll_bits.c'


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
    report = {'complete': False, 'source_sha256': hashlib.sha256(SOURCE.read_bytes()).hexdigest(),
              'arms': [], 'mutants': []}
    env = dict(os.environ)
    env.pop('CCC_NO_MAP_FP_UNROLL', None)
    modes = [('gcc', args.gcc, 'x86-64', {})]
    if args.clang:
        modes.append(('clang', args.clang, 'x86-64', {}))
    for profile, arch in [('sse2', 'x86-64'), ('sse41', 'x86-64-v2'), ('avx2', 'x86-64-v3')]:
        for rolled in [False, True]:
            modes.append((profile + ('-rolled' if rolled else ''), args.lccc, arch,
                          {'CCC_NO_MAP_FP_UNROLL': '1'} if rolled else {}))
    try:
        with tempfile.TemporaryDirectory(prefix='fp-map-unroll-') as td:
            for dtype, flags in [('f32', []), ('f64', ['-DFP_MAP_DOUBLE'])]:
                expected = None
                counts = {}
                for name, cc, arch, extra in modes:
                    arm_env = dict(env, **extra)
                    if cc == args.lccc:
                        arm_env['CCC_VALIDATE_SSA'] = '1'
                    binary = Path(td) / (dtype + '-' + name)
                    common = ['-O2', '-ffp-contract=off', '-march=' + arch, *flags]
                    cmd = [cc, *common, str(SOURCE), '-o', str(binary)]
                    run(cmd, arm_env)
                    output = run([str(binary)], arm_env)
                    if expected is None:
                        expected = output
                    if output != expected:
                        raise RuntimeError(f'{dtype}/{name}: floating bits or alias semantics differ from GCC')
                    row = {'name': dtype + '/' + name, 'command': cmd, 'env': extra,
                           'cases': len(output.splitlines()), 'sha256': hashlib.sha256(output).hexdigest()}
                    if cc == args.lccc:
                        asm = binary.with_suffix('.s')
                        run([cc, *common, '-S', str(SOURCE), '-o', str(asm)], arm_env)
                        match = re.search(r'(?ms)^checked_fp_map:\n(.*?)^\.size checked_fp_map,', asm.read_text())
                        if not match:
                            raise RuntimeError(f'{dtype}/{name}: missing checked_fp_map')
                        body = match[1]
                        suffix = 'ps' if dtype == 'f32' else 'pd'
                        count = len(re.findall(r'^\s+v?mul' + suffix + r'\s', body, re.M))
                        if re.search(r'\bvfm(?:add|sub)', body):
                            raise RuntimeError(f'{dtype}/{name}: illegal contraction with -ffp-contract=off')
                        row['packed_multiply_count'] = count
                        row['assembly_sha256'] = hashlib.sha256(body.encode()).hexdigest()
                        counts[name] = count
                    report['arms'].append(row)
                    print(f'{dtype}/{name}: PASS {row["cases"]} exact-bit alias/guard cases')
                for profile in ['sse2', 'sse41', 'avx2']:
                    if not counts[profile + '-rolled'] or counts[profile] != 2 * counts[profile + '-rolled']:
                        raise RuntimeError(f'{dtype}/{profile}: unroll ablation not observable: {counts}')
                # Keep non-default optimization pipelines honest without
                # demanding packed code when their policy disables vectorization.
                for opt in ['-O0', '-O1', '-O3', '-Os']:
                    for arch in ['x86-64', 'x86-64-v2', 'x86-64-v3']:
                        binary = Path(td) / (dtype + '-' + opt[1:] + '-' + arch)
                        cmd = [args.lccc, opt, '-ffp-contract=off', '-march=' + arch,
                               *flags, str(SOURCE), '-o', str(binary)]
                        checked_env = dict(env, CCC_VALIDATE_SSA='1')
                        run(cmd, checked_env)
                        output = run([str(binary)], checked_env)
                        if output != expected:
                            raise RuntimeError(f'{dtype}/{opt}/{arch}: exact bits differ from GCC')
                        report['arms'].append({'name': dtype + '/' + opt + '/' + arch,
                                               'command': cmd, 'env': {'CCC_VALIDATE_SSA': '1'},
                                               'cases': len(output.splitlines()),
                                               'sha256': hashlib.sha256(output).hexdigest()})
                # Real native fused arithmetic must disagree on the cancellation
                # witness: accepting it would silently weaken strict FP order.
                mutant = Path(td) / (dtype + '-fma-mutant')
                run([args.gcc, '-O2', '-march=x86-64-v3', '-ffp-contract=fast', *flags,
                     str(SOURCE), '-o', str(mutant)], env)
                actual = run([str(mutant)], env)
                if actual == expected:
                    raise RuntimeError(f'{dtype}: fused-arithmetic mutant survived')
                report['mutants'].append({'name': dtype + '/fma', 'killed': True,
                                          'sha256': hashlib.sha256(actual).hexdigest()})
            report['complete'] = True
    finally:
        if args.json:
            args.json.parent.mkdir(parents=True, exist_ok=True)
            args.json.write_text(json.dumps(report, indent=2) + '\n')


if __name__ == '__main__':
    main()
