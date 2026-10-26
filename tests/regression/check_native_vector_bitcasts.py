#!/usr/bin/env python3
"""Output/ABI oracle for native, full-width SIMD type-punning casts."""
import os
from pathlib import Path
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / 'tests/regression/native_vector_bitcasts.c'


def run(command):
    result = subprocess.run([str(x) for x in command], capture_output=True, timeout=60)
    if result.returncode:
        raise RuntimeError(f'{command}: exit={result.returncode}\n{result.stderr.decode(errors="replace")}')
    return result.stdout


def main():
    lccc = os.environ.get('CCC', str(ROOT / 'target/fastbuild/lccc'))
    gcc = os.environ.get('GCC', '/home/user/bin/gcc-oracle')
    modes = [('-m32', ('sse2',)), ('-m64', ('sse2', 'avx2'))]
    legs = 0
    with tempfile.TemporaryDirectory(prefix='native-vector-bitcast-') as tmp:
        tmp = Path(tmp)
        for mode, profiles in modes:
            for profile in profiles:
                flags = ['-O2', '-ffp-contract=off']
                flags += ['-msse2', '-mno-avx'] if profile == 'sse2' else ['-mavx2']
                for compiler in (gcc, lccc):
                    binary = tmp / f'{Path(compiler).name}-{mode}-{profile}'
                    include = ['-I' + str(ROOT / 'include')] if compiler == lccc else []
                    if compiler == lccc:
                        obj = tmp / f'lccc-{mode}-{profile}.o'
                        run([compiler, mode, *flags, *include, '-c', SOURCE, '-o', obj])
                        run([gcc, mode, obj, '-o', binary])  # Link both codegen results with the same current oracle.
                    else:
                        run([compiler, mode, *flags, *include, SOURCE, '-o', binary])
                    run([binary])  # The fixture's exact-bit checks and return code are the oracle.
                assembly = tmp / f'assembly-{mode}-{profile}.s'
                run([lccc, mode, *flags, '-I' + str(ROOT / 'include'), '-S', SOURCE, '-o', assembly])
                text = assembly.read_text()
                for symbol in ('cast128',) + (('cast256',) if profile == 'avx2' else ()):
                    body = text.split(symbol + ':', 1)[1].split('.size ' + symbol, 1)[0]
                    if mode == '-m64' and profile == 'sse2' and symbol == 'cast128':
                        instructions = [line for line in body.splitlines()
                                        if re.match(r'\s+[a-z][a-z0-9]*\s', line)]
                        if (len(instructions) > 3 or any('%rsp' in line or '%rbp' in line
                                                         for line in instructions)):
                            raise RuntimeError(f'{symbol}/{profile}: expected one XMM load/store and ret, got {instructions}')
                    # Reinterpretation must not become FP arithmetic or an
                    # out-of-line memcpy. The independent executable checks
                    # all bits, including SNaN/NaN payload and signed zero.
                    if re.search(r'\b(?:addps|subps|mulps|divps|sqrtps|addpd|subpd|mulpd|divpd|sqrtpd)\b', body):
                        raise RuntimeError(f'{symbol}/{profile}: type-pun became numeric FP arithmetic')
                    if re.search(r'\bcall\w*\s+.*(?:memcpy|memmove)', body):
                        raise RuntimeError(f'{symbol}/{profile}: type-pun lowered to an out-of-line copy')
                legs += 2
                print(f'PASS output/ISA/ABI {mode}/{profile}', flush=True)
    print(f'PASS: {legs} independent native vector-bitcast executable comparisons')


if __name__ == '__main__':
    main()
