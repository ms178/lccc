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


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--clang', default=os.environ.get('CLANG', 'clang-23'))
    p.add_argument('--lccc', default='target/fastbuild/lccc-x86')
    p.add_argument('--i686', default='target/fastbuild/lccc-i686')
    a = p.parse_args()
    version = subprocess.run([a.clang, '--version'], capture_output=True, text=True, check=True).stdout
    if not check_version(version):
        sys.exit('FAIL: require identified Clang 23.1.4, got '+version.splitlines()[0])
    print(version.splitlines()[0], flush=True)
    with tempfile.TemporaryDirectory(prefix='sse34-clang-') as d:
        d = Path(d)
        for bits, cc, path in [(64,a.lccc,'sse34-complete.casefile'),(32,a.i686,'i686/sse34-complete.casefile')]:
            cases = [c for c in asmdiff.load_cases(ROOT/'tests/asm-diff'/path) if c.mode != "reject"]
            if not cases:
                sys.exit('FAIL: empty positive matrix')
            source = d/f'matrix-{bits}.s'
            source.write_text('\n'.join(c.text for c in cases))
            lo, co = d/'lccc.o', d/'clang.o'
            for command in ([cc,'-c',str(source),'-o',str(lo)],
                            [a.clang,f'-m{bits}','-c','-x','assembler',str(source),'-o',str(co)]):
                subprocess.run(command, check=True, timeout=120)
            errors = asmdiff.compare(asmdiff.read_elf(lo), asmdiff.read_elf(co), check_symbols=False)
            if errors:
                sys.exit('FAIL: '+str(bits)+'-bit LLVM mismatch: '+'; '.join(errors))
            print(f'PASS: {bits}-bit SIMD matrix vs Clang: {len(cases)} positive groups, exact allocated bytes/relocations')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
