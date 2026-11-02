#!/usr/bin/env python3
"""Executable CET targets and prototype-section/alias contracts.

No kernel code is built by any oracle. Native GCC/Clang compare standalone
fixtures; LCCC additionally covers i686, AArch64 and RV64 with freestanding
QEMU execution. Missing target compilers or emulators are failures, not passes.
"""
from pathlib import Path
import argparse
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
STARTS = {
    'x86': 'call main\nmov %eax,%edi\nmov $60,%eax\nsyscall\n',
    'i686': 'call main\nmov %eax,%ebx\nmov $1,%eax\nint $0x80\n',
    'arm': 'bl main\nmov x8,#93\nsvc #0\n',
    'riscv': 'call main\nli a7,93\necall\n',
}
EXPECTED = {'early': '.init.text', 'early_alias': '.init.text',
            'typed': '.typed.text', 'local': '.local.text',
            'cold_function': '.init.text', 'repeated': '.repeated.text',
            'pointer_object': '.pointer.data'}


def run(argv):
    result = subprocess.run(list(map(str, argv)), capture_output=True, text=True, timeout=90)
    assert result.returncode == 0, (list(map(str, argv)), result.returncode, result.stdout, result.stderr)
    assert not result.stderr, (list(map(str, argv)), result.stderr)
    return result.stdout


def sections(obj):
    names = {}
    for line in run(['readelf', '-SW', obj]).splitlines():
        match = re.match(r'\s*\[\s*(\d+)\]\s+(\S+)\s', line)
        if match:
            names[match[1]] = match[2]
    symbols = {}
    for line in run(['readelf', '-sW', obj]).splitlines():
        fields = line.split()
        if len(fields) >= 8 and fields[0].rstrip(':').isdigit():
            symbols[fields[-1]] = (names.get(fields[-2], fields[-2]), fields[1], fields[4])
    for symbol, expected in EXPECTED.items():
        assert symbols.get(symbol, (None,))[0] == expected, (obj, symbol, expected, symbols.get(symbol))
    assert symbols['early'][1] == symbols['early_alias'][1], symbols
    assert symbols['early_alias'][2] == 'WEAK', symbols
    assert symbols['plain'][0] not in set(EXPECTED.values()), symbols


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--gcc', default=shutil.which('gcc-16') or shutil.which('gcc'))
    parser.add_argument('--clang', default=shutil.which('clang-23') or shutil.which('clang'))
    parser.add_argument('--icx', default=shutil.which('icx'))
    args = parser.parse_args()
    assert args.gcc and args.clang, 'GCC and Clang oracles are required'
    jobs = [(name, ROOT/f'target/fastbuild/lccc-{name}') for name in STARTS]
    jobs += [('gcc', Path(args.gcc)), ('clang', Path(args.clang))]
    if args.icx:
        jobs.append(('icx', Path(args.icx)))
    else:
        print('UNMEASURED: ICX oracle (not installed; not counted as passing)')
    checks = 0
    with tempfile.TemporaryDirectory(prefix='lccc-kernel-codegen-') as directory:
        work = Path(directory)
        for name, compiler in jobs:
            assert compiler.is_file(), compiler
            native = name not in ('i686', 'arm', 'riscv')
            architecture = 'x86' if native else name
            emulator = {'i686': 'qemu-i386', 'arm': 'qemu-aarch64', 'riscv': 'qemu-riscv64'}.get(name)
            if emulator:
                assert shutil.which(emulator), f'{emulator} required'
            start = work/f'{name}-start.S'
            start.write_text('.text\n.globl _start\n_start:\n' + STARTS[architecture] +
                             '.section .note.GNU-stack,"",%progbits\n')
            if name in ('gcc', 'clang', 'icx'):
                print(run([compiler, '--version']).splitlines()[0])
            for opt in ('-O0', '-O2', '-Os'):
                obj = work/f'{name}-sections.o'
                run([compiler, opt, '-c', HERE/'function_prototype_sections.c', '-o', obj])
                sections(obj)
                checks += 1
                section_runtime = work/f'{name}-sections-runtime'
                run([compiler, '-fno-pie', '-nostdlib', '-static', start, obj,
                     '-o', section_runtime])
                run(([emulator] if emulator else []) + [section_runtime])
                checks += 1
                modes = ('none', 'branch', 'full') if architecture in ('x86', 'i686') else (None,)
                for mode in modes:
                    output = work/f'{name}-runtime'
                    flags = [] if mode is None else [f'-fcf-protection={mode}']
                    enabled = mode in ('branch', 'full')
                    run([compiler, opt, '-fno-pie', '-nostdlib', '-static',
                         *flags, f'-DTEST_IBT={int(enabled)}', start,
                         HERE/'cet_computed_goto_targets.c', '-o', output])
                    run(([emulator] if emulator else []) + [output])
                    if architecture in ('x86', 'i686'):
                        disassembly = run(['objdump', '-d', '--disassemble=asm_only', output])
                        count = len(re.findall(r'\bendbr(?:32|64)\b', disassembly))
                        assert count == int(enabled), (name, opt, mode, disassembly)
                    checks += 1
            print(f'PASS {name}: sections, weak alias, computed goto, function entry, asm-goto')
        # GCC applies a section declared after the body, whereas Clang warns
        # and ignores it. Test this GNU-specific behavior separately rather
        # than hide that oracle disagreement in the common positive fixture.
        late = work/'late.c'
        late.write_text('int late(int x) { return x+2; }\n'
                        'int late(int) __attribute__((section(".exit.text")));\n')
        for name, compiler in jobs:
            if name in ('clang', 'icx'):
                continue
            obj = work/f'{name}-late.o'
            run([compiler, '-c', late, '-o', obj])
            assert '.exit.text' in run(['readelf', '-SW', obj]), (name, obj)
            checks += 1
    print(f'kernel codegen contracts: PASS ({checks} object/runtime contracts)')


if __name__ == '__main__':
    main()
