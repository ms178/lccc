#!/usr/bin/env python3
"""Exact packed shifts: native references, actual mechanisms and disable controls."""
import argparse
import json
import os
from pathlib import Path
import re
import tempfile
from check_llvm_missed_zero_choice import run, body
from check_llvm_predicate_relations import gcc_oracle_identity

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / 'tests/regression/packed_mask_shifts.c'


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--compiler', default=os.environ.get('CCC', str(ROOT / 'target/fastbuild/lccc')))
    p.add_argument('--gcc', default=os.environ.get('GCC', 'gcc-16'))
    p.add_argument('--clang', default=os.environ.get('CLANG'))
    p.add_argument('--baseline')
    p.add_argument('--allow-gcc16-snapshot', action='store_true')
    p.add_argument('--json')
    args = p.parse_args()
    version = run([args.gcc, '-dumpfullversion']).decode().strip()
    normative = gcc_oracle_identity(version, args.allow_gcc16_snapshot)
    if args.clang and run([args.clang, '-dumpversion']).decode().strip() != '23.1.4':
        raise ValueError('exact Clang 23.1.4 required')
    env = os.environ | {'CCC_VERIFY_IR': 'abort', 'CCC_VERIFY_REGALLOC': '1', 'CCC_VALIDATE_SSA': '1'}
    off = env | {'CCC_DISABLE_PASSES': 'vecrelations'}
    names = re.findall(r'void (packed_\w+)\(', SOURCE.read_text())
    if len(set(names)) != 52:
        raise ValueError('expected 52 kernels')
    receipts = []
    with tempfile.TemporaryDirectory() as tmp:
        directory = Path(tmp)
        for width, flags in [(128, ['-msse2', '-DONLY128']), (256, ['-march=x86-64-v3'])]:
            expected = None
            for label, compiler in [('gcc', args.gcc)] + ([('clang', args.clang)] if args.clang else []):
                exe = directory / label
                run([compiler, '-O2', *flags, '-Wall', '-Wextra', '-Wno-unused-variable', '-Werror', str(SOURCE), '-o', str(exe)])
                output = run([str(exe)])
                if expected is not None and output != expected:
                    raise ValueError('vendor disagreement')
                expected = output
            modes = [[level, *flags] for level in ('-O0', '-O1', '-O2', '-O3', '-Os')] + [['-O2', '-g', *flags]]
            if width == 128:
                modes += [[level, '-mno-sse2', '-msse', '-DONLY128'] for level in ('-O0', '-O2', '-Os')]
            for options in modes:
                print('native mode:', options, flush=True)
                exe = directory / 'native'
                run([args.compiler, *options, str(SOURCE), '-o', str(exe)], env=env)
                if run([str(exe)]) != expected:
                    raise ValueError(f'native mismatch: {options}')
            controls = [('disabled', args.compiler, off)]
            if args.baseline:
                controls += [('baseline', args.baseline, env)]
            for label, compiler, settings in controls:
                exe = directory / label
                run([compiler, '-O2', *flags, str(SOURCE), '-o', str(exe)], env=settings)
                if run([str(exe)]) != expected:
                    raise ValueError(f'{label} mismatch')
            asm = {}
            for label, compiler, settings in [('enabled', args.compiler, env), *controls]:
                path = directory / f'{label}.s'
                run([compiler, '-O2', *flags, '-S', str(SOURCE), '-o', str(path)], env=settings)
                asm[label] = path.read_text()
            active = [name for name in names if width == 256 or name.endswith('_128')]
            rows = [{'name': name, **{label: len(body(text, name)) for label, text in asm.items()}} for name in active]
            # The disabled compiler must retain redundant packed shifts; this
            # is a causal mechanism check, not only a correctness comparison.
            for bits in (16, 32):
                for kind in ('shl_chain', 'lshr_chain', 'ashr_chain', 'mask_direct', 'mask_reextend', 'mask_tree'):
                    name = f'packed_{kind}_{bits}_{width}'
                    row = next(r for r in rows if r['name'] == name)
                    if row['enabled'] >= row['disabled']:
                        raise ValueError(f'expected isolated improvement: {row}')
            for bits in (16, 32):
                for kind in ('mask_direct', 'mask_reextend', 'mask_tree'):
                    name = f'packed_{kind}_{bits}_{width}'
                    if any(re.match(r'v?ps(?:ll|ra)[wdq]\\b', line) for line in body(asm['enabled'], name)):
                        raise ValueError(f'canonical mask still shifted: {name}')
                for kind in ('nonmask_control', 'shared_control'):
                    row = next(r for r in rows if r['name'] == f'packed_{kind}_{bits}_{width}')
                    if row['enabled'] != row['disabled']:
                        raise ValueError(f'guard control changed: {row}')
            receipts.append({'width': width, 'checksum': expected.decode().strip(), 'comparisons': rows})
        run([args.compiler, '-O2', '-m32', '-msse2', '-DONLY128', '-DMASK_KERNELS_ONLY', '-S', str(SOURCE), '-o', str(directory / 'i686.s')], env=env)
    report = {'correct': True, 'native_kernels': 52, 'gcc_version': version, 'gcc_16_2_normative': normative,
              'clang_version': '23.1.4' if args.clang else None, 'receipts': receipts,
              'timing_claim': False, 'control': 'only vecrelations disabled; existing lowering, SSA and backend optimizations remain active'}
    if args.json:
        Path(args.json).write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({k: v for k, v in report.items() if k != 'receipts'}, sort_keys=True))


if __name__ == '__main__':
    main()
