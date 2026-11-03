#!/usr/bin/env python3
"""Non-kernel GCC16/Clang23 oracles for raw AVX FP bits and private frames.

All generated outputs must exit zero; NaNs/signed zeros are checked bitwise
inside the fixtures. O2/O3/Os additionally retain the strict 2048-byte frame
limit. The pre-fix compiler fails the original private-frame fixture at 4504
bytes, so this gate is mutation-sensitive without relying on assembly counts.
"""
from __future__ import annotations
import argparse
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = ('avx256_fp_operand_order', 'avx256_fp_private_frame')


def run(command):
    result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, timeout=30)
    if result.returncode or result.stderr:
        raise AssertionError(f'{command}: exit {result.returncode}\n{result.stdout}\n{result.stderr}')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lccc', default='target/fastbuild/lccc')
    parser.add_argument('--gcc', default=os.environ.get('GCC', 'gcc-16'))
    parser.add_argument('--clang', default=os.environ.get('CLANG', 'clang-23'))
    args = parser.parse_args()
    count = 0
    with tempfile.TemporaryDirectory(prefix='lccc-avx-fp-contracts-') as td:
        output = Path(td) / 'fixture'
        for label, compiler in [('LCCC', args.lccc), ('GCC16', args.gcc), ('Clang23', args.clang)]:
            for fixture in FIXTURES:
                source = ROOT / 'tests/regression' / (fixture + '.c')
                for opt in ('-O0', '-O1', '-O2', '-O3', '-Os'):
                    flags = [opt, '-mavx2', '-Wall', '-Wextra', '-Werror']
                    run([compiler, *flags, str(source), '-o', str(output)])
                    result = run([str(output)])
                    assert not result.stdout, result.stdout
                    count += 1
                    if opt in ('-O2', '-O3', '-Os'):
                        # Clang spells the severity group separately from its
                        # numeric threshold; GCC/LCCC accept the combined form.
                        strict = (['-Wframe-larger-than=2048', '-Werror=frame-larger-than=']
                                  if label == 'Clang23' else ['-Werror=frame-larger-than=2048'])
                        run([compiler, *flags, *strict, '-S', str(source), '-o', str(output)+'.s'])
                        count += 1
        # i686 executes the original, header-free frame fixture as a static
        # userspace process. No multilib libc or kernel code is involved.
        start = Path(td) / 'start.S'
        start.write_text('.text\n.globl _start\n.type _start,@function\n_start:\n'
                         'andl $-32, %esp\ncall main\nmovl %eax, %ebx\n'
                         'movl $1, %eax\nint $0x80\n'
                         # Clang O0 lowers constant aggregate initialization
                         # to memcpy even in a freestanding translation unit.
                         '.globl memcpy\n.type memcpy,@function\nmemcpy:\n'
                         'pushl %edi\npushl %esi\nmovl 12(%esp), %edi\n'
                         'movl 16(%esp), %esi\nmovl 20(%esp), %ecx\n'
                         'movl %edi, %eax\ncld\nrep movsb\npopl %esi\npopl %edi\nret\n'
                         '.size memcpy, .-memcpy\n'
                         '.section .note.GNU-stack,"",@progbits\n')
        source = ROOT / 'tests/regression/avx256_fp_private_frame.c'
        for label, compiler, target in [('LCCC', args.lccc + '-i686', []),
                                        ('GCC16', args.gcc, ['-m32']),
                                        ('Clang23', args.clang, ['-m32'])]:
            for opt in ('-O0', '-O1', '-O2', '-O3', '-Os'):
                flags = [*target, opt, '-mavx2', '-Wall', '-Wextra', '-Werror',
                         '-ffreestanding', '-fno-pie', '-fno-stack-protector']
                run([compiler, *flags, '-nostdlib', '-static', str(start),
                     str(source), '-o', str(output)])
                assert not run([str(output)]).stdout
                count += 1
                if opt in ('-O2', '-O3', '-Os'):
                    strict = (['-Wframe-larger-than=2048', '-Werror=frame-larger-than=']
                              if label == 'Clang23' else ['-Werror=frame-larger-than=2048'])
                    run([compiler, *flags, *strict, '-S', str(source), '-o', str(output)+'.s'])
                    count += 1
        print(f'AVX FP contracts: PASS ({count} build/runtime/frame checks; three compilers; x86-64/i686)')


if __name__ == '__main__':
    main()
