#!/usr/bin/env python3
"""Executable CET targets and prototype-section/alias contracts.

No kernel code is built by any oracle. Native GCC/Clang compare standalone
fixtures; LCCC additionally covers i686, AArch64 and RV64 with freestanding
QEMU execution. Missing target compilers or emulators are failures, not passes.
"""
from pathlib import Path
import argparse
import os
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


def return_thunk_flags(compiler):
    """Request the same return-thunk contract through each vendor's driver.

    ICX 2026.2 warns and ignores the GCC-style driver spelling. Forward the
    Clang cc1 option explicitly instead; the native gate must still prove
    support without warnings and satisfy the JMP/object/runtime assertions.
    """
    option = '-mfunction-return=thunk-extern'
    return ['-Xclang', option] if compiler == 'icx' else [option]


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
    parser.add_argument('--gcc', default=os.environ.get('GCC_ORACLE_BIN') or shutil.which('gcc-16') or shutil.which('gcc'))
    parser.add_argument('--clang', default=os.environ.get('CLANG') or shutil.which('clang-23') or shutil.which('clang'))
    parser.add_argument('--icx', default=os.environ.get('LCCC_ICX_BIN') or shutil.which('icx'))
    parser.add_argument('--require-icx', action='store_true')
    parser.add_argument('--native-only', action='store_true',
                        help='native oracle lane; default still requires all four LCCC targets')
    args = parser.parse_args()
    assert args.gcc and args.clang, 'GCC and Clang oracles are required'
    assert not args.require_icx or args.icx, 'authenticated ICX oracle required'
    targets = ('x86',) if args.native_only else STARTS
    jobs = [(name, ROOT/f'target/fastbuild/lccc-{name}') for name in targets]
    jobs += [('gcc', Path(shutil.which(args.gcc) or args.gcc)),
             ('clang', Path(shutil.which(args.clang) or args.clang))]
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
                version = run([compiler, '--version']).splitlines()[0]
                print(version)
                if name == 'gcc':
                    assert re.search(r'\b16\.2\.0\b', version), ('expected reviewed GCC 16.2', version)
                if name == 'icx':
                    assert re.search(r'\b2026\.2(?:\.\d+)?\b', version), ('expected reviewed ICX 2026.2', version)
                if name == 'clang':
                    assert re.search(r'clang version 23\.1\.4\b', version), ('expected reviewed Clang 23.1.4', version)
            # Standalone return-transfer ABI fixture, not Linux kernel code.
            # Linux's runtime patcher requires JMP sites, not Jcc-to-thunk.
            if architecture in ('x86', 'i686'):
                thunk_source = work / 'return-transfer.c'
                thunk_source.write_text(
                    'volatile unsigned enabled;\n'
                    '__attribute__((noinline)) void conditional_return(unsigned x) {\n'
                    ' if (x < enabled) return; __asm__ volatile("nop" ::: "memory"); }\n'
                    '__asm__(".text\\n.globl __x86_return_thunk\\n" '
                    '"__x86_return_thunk: ret\\n");\n'
                    'int main(void) { enabled=2; conditional_return(0); '
                    'conditional_return(3); return 0; }\n')
                if name == 'i686':
                    # Callee cleanup must preserve return lanes and the caller
                    # stack, including the extension that returns in ECX.
                    thunk_source.write_text(thunk_source.read_text().replace(
                        'int main(void)', 'int simple_main(void)') +
                        '__asm__(".text\\n.globl __x86_return_thunk_ecx\\n" '
                        '"__x86_return_thunk_ecx: pushl %ecx; ret\\n");\n'
                        'struct Four { int a,b,c,d; };\n'
                        '__attribute__((noinline)) struct Four record(int x) { '
                        'return (struct Four){x,x+1,x+2,x+3}; }\n'
                        '__attribute__((noinline,fastcall)) long long wide(int a,int b,int c,int d) { '
                        'return ((long long)(unsigned)(a+b)<<32) | (unsigned)(c+d); }\n'
                        'typedef int V __attribute__((ext_vector_type(3)));\n'
                        '__attribute__((noinline,fastcall)) V vector(int a,int b,int c,int d) { '
                        'return (V){a+b,c,d}; }\n'
                        'int main(void) { struct Four r=record(5); '
                        'long long w=wide(1,2,3,4); V v=vector(1,2,3,4); '
                        'return simple_main() || r.a!=5 || r.d!=8 || '
                        'w!=(((long long)3<<32)|7) || v[0]!=3 || v[1]!=3 || v[2]!=4; }\n')
                for opt in ('-O0', '-O2', '-Os'):
                    assembly = work / f'{name}-return.s'
                    flags = [opt, *return_thunk_flags(name), '-fcf-protection=branch']
                    run([compiler, *flags, '-S', thunk_source, '-o', assembly])
                    text = assembly.read_text()
                    assert not re.search(r'^\s*j(?!mp\b)\w+\s+__x86_return_thunk', text, re.M), (name, opt, text)
                    assert re.search(r'^\s*jmp\s+__x86_return_thunk', text, re.M), (name, opt, text)
                    output = work / f'{name}-return-runtime'
                    run([compiler, *flags, '-fno-pie', '-nostdlib', '-static',
                         start, thunk_source, '-o', output])
                    run(([emulator] if emulator else []) + [output])
                    checks += 2
            # Mutable global callback cells used to bypass retpolines through
            # the RIP-relative load/call fold. Inspect object code (not just
            # assembly spelling) and execute stack-argument calls through real
            # speculation-trapping thunks. These are not Linux target sources.
            if architecture == 'x86':
                thunks = work / 'indirect-thunks.S'
                definitions = ['.text']
                for reg in ('rax', 'rbx', 'rcx', 'rdx', 'rsi', 'rdi', 'rbp',
                            'r8', 'r9', 'r10', 'r11', 'r12', 'r13', 'r14', 'r15'):
                    definitions += [f'.globl __x86_indirect_thunk_{reg}',
                                    f'__x86_indirect_thunk_{reg}:',
                                    'call 2f', '1: pause', 'lfence', 'jmp 1b',
                                    f'2: mov %{reg},(%rsp)', 'ret']
                definitions += ['.section .note.GNU-stack,"",@progbits']
                thunks.write_text('\n'.join(definitions) + '\n')
                got_source = work / 'got-variadic.c'
                got_source.write_text(
                    'extern double external_v(double, int, ...);\n'
                    'int main(void) { return external_v(1.5, 7, 3.25) != 11.75; }\n')
                got_impl = work / 'got-variadic-impl.c'
                got_impl.write_text(
                    '#include <stdarg.h>\n'
                    'double external_v(double first, int n, ...) {\n'
                    'va_list ap; va_start(ap, n); double last = va_arg(ap, double);\n'
                    'va_end(ap); return first + n + last; }\n')
                got_impl_obj = work / f'{name}-got-impl.o'
                run([compiler, '-O2', '-fno-pie', '-c', got_impl, '-o', got_impl_obj])
                for opt in ('-O0', '-O2', '-Os'):
                    for mode in ('external', 'inline', 'none'):
                        if name in ('clang', 'icx'):
                            branch_flags = {'external': ['-mretpoline-external-thunk'],
                                            'inline': ['-mretpoline'], 'none': []}[mode]
                        else:
                            branch_flags = ([] if mode == 'none' else
                                            ['-mindirect-branch=thunk-' +
                                             ('extern' if mode == 'external' else 'inline')])
                        obj = work / f'{name}-{opt}-{mode}-global.o'
                        run([compiler, opt, '-fno-pie', *branch_flags, '-c',
                             HERE / 'global_indirect_thunks.c', '-o', obj])
                        disassembly = run(['objdump', '-dr', obj])
                        raw = re.search(r'\b(?:call|jmp)q?\s+\*', disassembly)
                        assert bool(raw) == (mode == 'none'), (name, opt, mode, disassembly)
                        if mode == 'external':
                            assert '__x86_indirect_thunk_' in disassembly, disassembly
                        output = work / f'{name}-{opt}-{mode}-global-runtime'
                        run([compiler, '-fno-pie', '-nostdlib', '-static',
                             start, obj, thunks, '-o', output])
                        run([output])
                        checks += 2
                        got_obj = work / f'{name}-{opt}-{mode}-got.o'
                        run([compiler, opt, '-fno-pie', '-fno-plt', *branch_flags,
                             '-c', got_source, '-o', got_obj])
                        got_disassembly = run(['objdump', '-dr', got_obj])
                        got_raw = re.search(r'\b(?:call|jmp)q?\s+\*', got_disassembly)
                        assert bool(got_raw) == (mode == 'none'), (name, opt, mode, got_disassembly)
                        got_output = work / f'{name}-{opt}-{mode}-got-runtime'
                        run([compiler, '-fno-pie', '-nostdlib', '-static', start,
                             got_obj, got_impl_obj, thunks, '-o', got_output])
                        run([got_output])
                        checks += 2
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
                bool_runtime = work / f'{name}-{opt}-boolean-phi-cast'
                run([compiler, opt, '-fno-pie', '-nostdlib', '-static', start,
                     HERE / 'boolean_phi_cast_branch.c', '-o', bool_runtime])
                run(([emulator] if emulator else []) + [bool_runtime])
                checks += 1
                asm_callback = work / f'{name}-{opt}-asm-callback-entry'
                run([compiler, opt, '-fno-pie', '-nostdlib', '-static', start,
                     HERE / 'asm_callback_entry.c', '-o', asm_callback])
                run(([emulator] if emulator else []) + [asm_callback])
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
