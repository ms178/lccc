#!/usr/bin/env python3
"""Packed public arithmetic: exact bits and caller/provider ABI against GCC 16.2."""
import argparse
import os
import re
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / 'tests/regression/legacy_sse_native_chain.c'


def run(command):
    result = subprocess.run([str(x) for x in command], capture_output=True, timeout=90)
    if result.returncode:
        raise RuntimeError(f'{command}: exit {result.returncode}\n{result.stderr.decode(errors="replace")}')
    return result.stdout


def host_links_i386(gcc):
    """True when the oracle can link a `-m32` program on this host.

    The legs below *execute* 32-bit binaries and link them with the oracle, so
    a host without the 32-bit startup files and libc cannot measure them at
    all. Skipping them is the honest answer; pretending they passed is not,
    and blaming lccc for them is worse -- measured on the campaign image,
    where `gcc-16 -m32` stops at `cannot find crti.o`.
    """
    with tempfile.TemporaryDirectory(prefix='i386-link-probe-') as directory:
        source = Path(directory) / 'probe.c'
        binary = Path(directory) / 'probe'
        source.write_text('int main(void) { return 0; }\n')
        try:
            proc = subprocess.run([str(gcc), '-m32', str(source), '-o', str(binary)],
                                  capture_output=True, timeout=90)
        except (OSError, subprocess.TimeoutExpired):
            return False
        return proc.returncode == 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lccc', default=os.environ.get('CCC', str(ROOT / 'target/fastbuild/lccc')))
    parser.add_argument('--gcc', default=os.environ.get('GCC', shutil.which('gcc-16') or 'gcc-16'))
    parser.add_argument('--source', type=Path, default=SOURCE)
    parser.add_argument('--check-codegen', action='store_true', help='pin native_packed_fp_helpers.c provider density')
    args = parser.parse_args()
    if run([args.gcc, '-dumpfullversion']).strip() != b'16.2.0':
        raise RuntimeError('GCC 16.2.0 is required')
    profiles = {'sse1': ['-mno-sse2', '-msse', '-DSSE1_ONLY'],
                'sse2': ['-msse2', '-mno-avx'],
                'avx': ['-mavx', '-mno-avx2', '-DUSE_AVX'],
                'avx2': ['-mavx2', '-DUSE_AVX']}
    legs = 0
    with tempfile.TemporaryDirectory(prefix='legacy-sse-native-') as directory:
        directory = Path(directory)
        if args.check_codegen:
            # A mechanism gate, not a runtime speed claim. Baseline providers
            # have 70/77 SSE2 and 235/235 AVX2 instructions respectively.
            for flags, ceilings in [(['-msse2', '-mno-avx'], (69, 77)),
                                    (['-mavx2', '-DUSE_AVX'], (234, 234))]:
                assembly = directory / 'provider.s'
                run([args.lccc, '-m64', '-O2', *flags, '-DPROVIDER_ONLY',
                     '-S', args.source, '-o', assembly])
                text = assembly.read_text()
                for name, ceiling in zip(('helpers_f', 'helpers_d'), ceilings):
                    body = text.split(name + ':', 1)[1].split('.size ' + name, 1)[0]
                    count = sum(bool(re.match(r'[a-z][a-z0-9]*(?:\s|$)', line.strip()))
                                for line in body.splitlines())
                    if not 0 < count <= ceiling:
                        raise RuntimeError(f'{name}/{flags}: {count} instructions exceeds {ceiling}')
                    print(f'PASS codegen {name}/{flags}: {count} <= {ceiling}', flush=True)
        can_link_i386 = host_links_i386(args.gcc)
        if not can_link_i386:
            print('  skip -m32: this host cannot link or run 32-bit programs '
                  '(the oracle fails the same link)', flush=True)
        for mode in ('-m32', '-m64'):
            if mode == '-m32' and not can_link_i386:
                continue
            for opt in ('-O0', '-O2'):
                for profile, flags in profiles.items():
                    base = [mode, opt, '-ffp-contract=off', *flags]
                    for label, compiler in [('gcc', args.gcc), ('lccc', args.lccc)]:
                        for part in ('caller', 'provider'):
                            obj = directory / f'{label}-{part}.o'
                            define = '-DCALLER_ONLY' if part == 'caller' else '-DPROVIDER_ONLY'
                            run([compiler, *base, define, '-c', args.source, '-o', obj])
                    expected = None
                    for caller, provider in [('gcc', 'gcc'), ('gcc', 'lccc'),
                                             ('lccc', 'gcc'), ('lccc', 'lccc')]:
                        binary = directory / 'probe'
                        run([args.gcc, mode, directory / f'{caller}-caller.o',
                             directory / f'{provider}-provider.o', '-o', binary])
                        output = run([binary])
                        if expected is None:
                            expected = output
                        if output != expected:
                            raise RuntimeError(f'{mode}/{opt}/{profile}/{caller}->{provider}: '
                                               f'{output!r} != GCC {expected!r}')
                        legs += 1
                    print(f'PASS {mode} {opt} {profile}: all four caller/provider directions', flush=True)
    if can_link_i386:
        print(f'PASS: {legs} native packed arithmetic ABI/runtime legs, zero skips')
    else:
        print(f'PASS: {legs} native packed arithmetic ABI/runtime legs (i686 legs skipped: '
              f'this host cannot link 32-bit programs)')
    # Exercise the separate native type-punning/cross-family vector-SSA
    # contract in this already measured SIMD gate (no additional CI shard).
    checker = ROOT / 'tests/regression/check_native_vector_bitcasts.py'
    gcc_path = shutil.which(args.gcc) or str(Path(args.gcc).resolve())
    env = dict(os.environ, CCC=str(Path(args.lccc).resolve()), GCC=gcc_path)
    result = subprocess.run([os.environ.get('PYTHON', 'python3'), str(checker)],
                            capture_output=True, text=True, env=env, timeout=180)
    if result.returncode:
        raise RuntimeError(f'native vector bitcast regression failed\n{result.stdout}\n{result.stderr}')
    print(result.stdout, end='', flush=True)


if __name__ == '__main__':
    main()
