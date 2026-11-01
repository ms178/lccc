#!/usr/bin/env python3
"""Output/ABI oracle for native, full-width SIMD type-punning casts."""
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / 'tests/regression/native_vector_bitcasts.c'


def run(command):
    result = subprocess.run([str(x) for x in command], capture_output=True, timeout=60)
    if result.returncode:
        raise RuntimeError(f'{command}: exit={result.returncode}\n{result.stderr.decode(errors="replace")}')
    return result.stdout


def host_links_i386(gcc):
    """True when the oracle can link a `-m32` program here.

    The `-m32` legs link and then *run* 32-bit executables, so a host without
    the 32-bit startup files cannot measure them. Skipping them is honest;
    reporting them as failures blames lccc for the image.
    """
    with tempfile.TemporaryDirectory(prefix='i386-link-probe-') as tmp:
        source = Path(tmp) / 'probe.c'
        source.write_text('int main(void) { return 0; }\n')
        try:
            proc = subprocess.run([str(gcc), '-m32', str(source), '-o', str(Path(tmp) / 'probe')],
                                  capture_output=True, timeout=90)
        except (OSError, subprocess.TimeoutExpired):
            return False
        return proc.returncode == 0


def main():
    lccc = os.environ.get('CCC', str(ROOT / 'target/fastbuild/lccc'))
    # `$GCC` is how the CI harness names the pinned oracle. Falling back to a
    # hard-coded home directory made this gate unrunnable anywhere else, and a
    # silent `-m32`-less default would have hidden the fact that nothing ran.
    gcc = os.environ.get('GCC') or shutil.which('gcc-16') or shutil.which('gcc')
    if not gcc:
        raise SystemExit('check_native_vector_bitcasts: no gcc oracle found (set GCC)')
    modes = [('-m32', ('sse2',)), ('-m64', ('sse2', 'avx2'))]
    can_link_i386 = host_links_i386(gcc)
    legs = 0
    with tempfile.TemporaryDirectory(prefix='native-vector-bitcast-') as tmp:
        tmp = Path(tmp)
        for mode, profiles in modes:
            if mode == '-m32' and not can_link_i386:
                print('  skip -m32: this host cannot link or run 32-bit programs', flush=True)
                continue
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
    if can_link_i386:
        print(f'PASS: {legs} independent native vector-bitcast executable comparisons')
    else:
        print(f'PASS: {legs} independent native vector-bitcast executable comparisons '
              f'(i686 legs skipped: this host cannot link 32-bit programs)')


if __name__ == '__main__':
    main()
