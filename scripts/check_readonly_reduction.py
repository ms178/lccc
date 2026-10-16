#!/usr/bin/env python3
"""Guard-page/SSA/native/Memcheck checks for strided byte-load reductions.

The i686 Memcheck arm deliberately uses baseline SSE2: Valgrind 3.24 does
not decode AVX in its 32-bit frontend. It is not labeled as a v3 test.
"""
import argparse
import hashlib
import json
import os
import shutil
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lccc', default=os.environ.get('CCC', 'target/fastbuild/lccc'))
    parser.add_argument('--gcc')
    parser.add_argument('--clang')
    parser.add_argument('--json', type=Path, required=True)
    args = parser.parse_args()
    report = dict(complete=False, failures=[], cases=[])
    arms = [('lccc', args.lccc, '-O2', {}), ('lccc-O3', args.lccc, '-O3', {})]
    for label, compiler in [('gcc', args.gcc), ('clang', args.clang)]:
        if compiler:
            arms.append((label, compiler, '-O2', {}))
    try:
        with tempfile.TemporaryDirectory(prefix='lccc-readonly-guard-') as tmp:
            for label, compiler, opt, extra in arms:
                resolved = shutil.which(compiler)
                if resolved is None:
                    raise FileNotFoundError(f'compiler is not executable or not on PATH: {compiler}')
                cc = str(Path(resolved).resolve())
                identity = subprocess.check_output([cc, '--version'], text=True)
                for abi, flags in [('x86-64-v3', ['-march=x86-64-v3']),
                                   ('i686-SSE2', ['-m32', '-march=i686', '-msse2'])]:
                    binary = Path(tmp) / (label + '-' + abi)
                    command = [cc, opt, '-fwrapv', *flags,
                               str(ROOT / 'tests/regression/readonly_reduction_guard.c'), '-o', str(binary)]
                    env = dict(os.environ, **extra)
                    if compiler == args.lccc:
                        env['CCC_VALIDATE_SSA'] = '1'
                    build = subprocess.run(command, env=env, text=True, capture_output=True, timeout=60)
                    row = dict(label=label, abi=abi, command=command, environment=extra,
                               compiler_sha256=hashlib.sha256(Path(cc).read_bytes()).hexdigest(),
                               compiler_version=identity, build_status=build.returncode,
                               build_log=build.stdout + build.stderr)
                    report['cases'].append(row)
                    if build.returncode:
                        raise RuntimeError(f'{label}/{abi}: compile failed')
                    native = subprocess.run([str(binary)], text=True, capture_output=True, timeout=15)
                    vg = subprocess.run(['valgrind', '--tool=memcheck', '--error-exitcode=97',
                                         '--leak-check=full', str(binary)], text=True, capture_output=True, timeout=45)
                    row.update(native_status=native.returncode, stdout=native.stdout,
                               memcheck_status=vg.returncode, memcheck_log=vg.stderr)
                    if native.returncode or vg.returncode or native.stdout != 'd3a1ee5f\n' or vg.stdout != native.stdout:
                        raise RuntimeError(f'{label}/{abi}: native/Memcheck/checksum failed')
                    print(f'PASS {label}/{abi}: 33 boundary/seed cases', flush=True)
            report['complete'] = True
    except Exception as exc:
        report['failures'].append(str(exc))
        raise
    finally:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps(report, indent=2) + '\n')


if __name__ == '__main__':
    main()
