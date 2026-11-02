#!/usr/bin/env python3
"""Strict two-compiler public SIMD ABI grid (both directions and controls)."""
import argparse
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / 'tests/regression/public_simd_abi/boundary.c'


def run(command):
    subprocess.run([str(arg) for arg in command], check=True, timeout=120)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lccc', default=ROOT / 'target/fastbuild/lccc')
    parser.add_argument('--gcc', default=os.environ.get('GCC', 'gcc-16'))
    parser.add_argument('--target', choices=('m64', 'm32'), action='append', help='Targeted local subset; default tests both ABIs')
    args = parser.parse_args()
    gcc = args.gcc
    version = subprocess.check_output([gcc, '-dumpfullversion'], text=True).strip()
    if version != '16.2.0':
        parser.error(f'GCC 16.2.0 required, got {version}')
    compilers = {'lccc': str(Path(args.lccc).resolve()), 'gcc': gcc}
    count = 0
    with tempfile.TemporaryDirectory(prefix='public-simd-abi-') as tmp:
        tmp = Path(tmp)
        for mode in args.target or ('m64', 'm32'):
            target = '-' + mode
            # __m64 (BITS=64) is one type. On i686, GCC passes it in MMX
            # registers unless -mno-mmx; lccc never enables MMX, so both
            # compilers get -mno-mmx there (GCC's documented psABI variant).
            mmx = ['-mno-mmx'] if mode == 'm32' else []
            for bits, isa, kinds in ((64, ['-msse4.2', *mmx], 1), (128, ['-msse4.2'], 3),
                                     (256, ['-mavx', '-mno-avx2'], 3), (256, ['-mavx2'], 3)):
                for kind in range(kinds):
                    for opt in ('-O0', '-O2'):
                        flags = [target, opt, *isa, f'-DBITS={bits}', f'-DKIND={kind}']
                        objects = {}
                        for name, compiler in compilers.items():
                            for provider in (0, 1):
                                obj = tmp / f'{name}-{provider}.o'
                                run([compiler, *flags, f'-DPROVIDER={provider}', '-c', SOURCE, '-o', obj])
                                objects[name, provider] = obj
                        for caller in compilers:
                            for provider in compilers:
                                binary = tmp / 'boundary'
                                # Use the exact current GCC runtime/linker driver
                                # for every leg, including both compiler controls.
                                run([gcc, target, objects[caller, 0], objects[provider, 1], '-o', binary])
                                print(f'ABI {target} {isa} kind={kind} {opt}: {caller} -> {provider}', flush=True)
                                run([binary])
                                count += 1
    print(f'public native SIMD ABI: {count} executable legs PASS; zero skips')


if __name__ == '__main__':
    main()
