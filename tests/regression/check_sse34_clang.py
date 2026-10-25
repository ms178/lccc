#!/usr/bin/env python3
"""Independently compare all positive legacy SIMD matrix bytes with Clang 23.1.4.

GAS rejection-policy tests remain in asmdiff; LLVM need not copy GAS's syntax
policy for malformed input. This gate never treats a missing/old LLVM as a pass.
"""
import argparse
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT/'scripts'))
import asmdiff


def check_version(text):
    return bool(re.search(r'\bclang version 23\.1\.4(?:\s|$)', text))


def fail(message):
    if os.environ.get('GITHUB_ACTIONS') == 'true':
        escaped = message.replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A')
        print('::error::'+escaped, file=sys.stderr)
    raise SystemExit(message)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--clang', default=os.environ.get('CLANG', 'clang-23'))
    p.add_argument('--lccc', default='target/fastbuild/lccc-x86')
    p.add_argument('--i686', default='target/fastbuild/lccc-i686')
    a = p.parse_args()
    version = subprocess.run([a.clang, '--version'], capture_output=True, text=True, check=True).stdout
    if not check_version(version):
        fail('FAIL: require identified Clang 23.1.4, got '+version.splitlines()[0])
    print(version.splitlines()[0], flush=True)
    with tempfile.TemporaryDirectory(prefix='sse34-clang-') as d:
        d = Path(d)
        for bits, cc, path in [(64,a.lccc,'sse34-complete.casefile'),(32,a.i686,'i686/sse34-complete.casefile')]:
            cases = [c for c in asmdiff.load_cases(ROOT/'tests/asm-diff'/path) if c.mode != "reject"]
            if not cases:
                fail('FAIL: empty positive matrix')
            source = d/f'matrix-{bits}.s'
            source.write_text('\n'.join(c.text for c in cases))
            lo, co = d/'lccc.o', d/'clang.o'
            for command in ([cc,f'-m{bits}','-c',str(source),'-o',str(lo)],
                            [a.clang,f'-m{bits}','-c','-x','assembler',str(source),'-o',str(co)]):
                result = subprocess.run(command, capture_output=True, text=True, timeout=120)
                if result.returncode:
                    fail('FAIL: '+str(bits)+'-bit assembler '+command[0]+':\n'+(result.stderr or result.stdout)[-6000:])
            errors = asmdiff.compare(asmdiff.read_elf(lo), asmdiff.read_elf(co), check_symbols=False)
            if errors:
                fail('FAIL: '+str(bits)+'-bit LLVM mismatch: '+'; '.join(errors))
            print(f'PASS: {bits}-bit SIMD matrix vs Clang: {len(cases)} positive groups, exact allocated bytes/relocations')
    return 0


if __name__ == '__main__':
    try:
        raise SystemExit(main())
    except (OSError, subprocess.SubprocessError) as error:
        fail('FAIL: current LLVM comparison could not run: '+str(error))
