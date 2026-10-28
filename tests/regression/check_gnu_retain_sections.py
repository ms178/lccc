#!/usr/bin/env python3
"""SHF_GNU_RETAIN survives assembly and roots GC; C TLS retain is executable.

Tests x86-64, i686, AArch64, RV64 and RV32 integrated assemblers, comparing
native flags with GNU as; optional cross assemblers extend the oracle check.
Retained/non-retained same-name sections must either be represented correctly
or rejected: silently merging them changes --gc-sections semantics.
"""
from __future__ import annotations

import argparse
import os
from pathlib import Path
import struct
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
R = 0x200000
A, W, X, T = 2, 1, 4, 0x400
ASM = r'''
.text
.globl _start
_start: ret
.section .ret_data,"awR",@progbits
.globl ret_data
ret_data: .long 0x12345678
.section .ret_data,"awR"
.globl ret_data_tail
ret_data_tail: .long 4
.section .ret_ro,"aR",@progbits
.globl ret_ro
ret_ro: .long 11
.section .ret_bss,"awR",@nobits
.globl ret_bss
ret_bss: .zero 4
.section .ret_bss,"awR",@nobits
.globl ret_bss_tail
ret_bss_tail: .zero 4
.section .ret_percent,"awR",%nobits
.globl ret_percent
ret_percent: .zero 4
.section .ret_tls,"awTR",@progbits
.globl ret_tls
.type ret_tls,@tls_object
ret_tls: .long 5
.section .ret_text,"axR",@progbits
.globl ret_text
ret_text: ret
.pushsection .ret_link,"awR",@progbits
.globl ret_link
ret_link: .long dependency
.popsection
.section .dependency,"aw",@progbits
.globl dependency
dependency: .long 6
.section .drop_data,"aw",@progbits
.globl drop_data
drop_data: .long 3
.section .drop_text,"ax",@progbits
.globl drop_text
drop_text: ret
'''
EXPECTED = {
    '.ret_data': (1, A | W | R), '.ret_ro': (1, A | R),
    '.ret_bss': (8, A | W | R), '.ret_percent': (8, A | W | R),
    '.ret_tls': (1, A | W | T | R),
    '.ret_text': (1, A | X | R), '.ret_link': (1, A | W | R),
    '.dependency': (1, A | W), '.drop_data': (1, A | W),
    '.drop_text': (1, A | X),
}
RETAINED = {'ret_data', 'ret_data_tail', 'ret_ro', 'ret_bss', 'ret_bss_tail', 'ret_percent',
            'ret_tls', 'ret_text', 'ret_link', 'dependency'}
DISCARDED = {'drop_data', 'drop_text'}


def run(args: list[str], *, check: bool = True) -> subprocess.CompletedProcess:
    try:
        proc = subprocess.run([str(a) for a in args], capture_output=True, text=True, timeout=30)
    except (OSError, subprocess.TimeoutExpired) as error:
        raise RuntimeError(f'cannot run {args}: {error}') from error
    if check and proc.returncode:
        raise RuntimeError(f'{args} failed ({proc.returncode}):\n{proc.stderr}')
    return proc


def elf(path: Path) -> tuple[list[tuple[str, int, int]], dict[str, int]]:
    """Read names/types/flags and symbol section indices from a little-endian ELF."""
    data = path.read_bytes()
    if data[:4] != b'\x7fELF' or data[5] != 1:
        raise RuntimeError(f'{path}: expected little-endian ELF')
    if data[4] == 2:
        shoff = struct.unpack_from('<Q', data, 40)[0]
        entsize, count, strings = struct.unpack_from('<HHH', data, 58)
        fmt, symfmt = '<IIQQQQIIQQ', '<IBBHQQ'
    elif data[4] == 1:
        shoff = struct.unpack_from('<I', data, 32)[0]
        entsize, count, strings = struct.unpack_from('<HHH', data, 46)
        fmt, symfmt = '<IIIIIIIIII', '<IIIBBH'
    else:
        raise RuntimeError(f'{path}: unknown ELF class')
    headers = [struct.unpack_from(fmt, data, shoff + i * entsize) for i in range(count)]
    def blob(h: tuple) -> bytes:
        return data[h[4]:h[4] + h[5]]
    def cstring(buf: bytes, start: int) -> str:
        end = buf.index(b'\0', start)
        return buf[start:end].decode('utf-8')
    shstrings = blob(headers[strings])
    sections = [(cstring(shstrings, h[0]), h[1], h[2]) for h in headers]
    symbols = {}
    for h in headers:
        if h[1] != 2 or not h[9]:  # SHT_SYMTAB
            continue
        strtab = blob(headers[h[6]])
        for off in range(h[4], h[4] + h[5], h[9]):
            sym = struct.unpack_from(symfmt, data, off)
            if sym[0]:
                symbols[cstring(strtab, sym[0])] = sym[3] if data[4] == 2 else sym[5]
    return sections, symbols


def check_sections(obj: Path, expected: dict[str, tuple[int, int]]) -> None:
    sections, _ = elf(obj)
    if any(flags & R for _, flags in expected.values()) and obj.read_bytes()[7] != 3:
        raise RuntimeError(f'{obj}: retained sections require ELFOSABI_GNU for GNU ld')
    for name, want in expected.items():
        found = [(ty, flags) for n, ty, flags in sections if n == name]
        if found != [want]:
            raise RuntimeError(f'{obj}: {name}: got {found!r}, expected {[want]!r}')


TLS_SECTIONS = {
    '.tdata.retained_init': (1, A | W | T | R),
    '.tbss.retained_zero': (8, A | W | T | R),
    '.tdata.retained_const': (1, A | W | T | R),
    '.tbss.retained_const_zero': (8, A | W | T | R),
    '.custom_tls_init': (1, A | W | T | R),
    '.custom_tls_zero': (1, A | W | T | R),
    '.plain_tls_init': (1, A | W | T),
    '.plain_tls_zero': (1, A | W | T),
    '.data.rel.ro.retained_ptr': (1, A | W | R),
}
TLS_GLOBALS = {
    'retained_init': '.tdata.retained_init',
    'retained_zero': '.tbss.retained_zero',
    'retained_const': '.tdata.retained_const',
    'retained_const_zero': '.tbss.retained_const_zero',
    'custom_init': '.custom_tls_init',
    'custom_zero': '.custom_tls_zero',
    'plain_init': '.plain_tls_init',
    'plain_zero': '.plain_tls_zero',
}


def check_tls_source_object(obj: Path) -> None:
    """Check symbol *types* as well as section bits (both affect TLS ABI)."""
    check_sections(obj, TLS_SECTIONS)
    sections, symbols = elf(obj)
    types = {}
    for line in run(['readelf', '-sW', obj]).stdout.splitlines():
        fields = line.split()
        if len(fields) >= 8 and fields[0].endswith(':'):
            types[fields[7]] = fields[3]
    for name, section in TLS_GLOBALS.items():
        index = symbols.get(name, 0)
        if not 0 < index < len(sections) or sections[index][0] != section or types.get(name) != 'TLS':
            raise RuntimeError(f'{obj}: {name} is not TLS in {section}: '
                               f'index={index}, type={types.get(name)}')
    index = symbols.get('retained_ptr', 0)
    if not 0 < index < len(sections) or sections[index][0] != '.data.rel.ro.retained_ptr':
        raise RuntimeError(f'{obj}: retained PIC pointer is not in .data.rel.ro')
    # Local symbol suffixes are compiler-dependent; used prevents frontend DCE.
    for stem, wanted, kind, section_type in [
            ('local_thread.', A | W | T | R, 'TLS', 1),
            ('local_zero.', A | W | T | R, 'TLS', 8),
            ('local_object.', A | W | R, 'OBJECT', 1),
            ('local_spec_a.', A | W | R, 'OBJECT', 1),
            ('local_spec_b.', A | W | R, 'OBJECT', 1),
            ('local_sibling.', A | W, 'OBJECT', 1),
            ('local_after.', A | W, 'OBJECT', 1)]:
        found = [name for name in symbols if stem in name and types.get(name) == kind]
        if len(found) != 1:
            raise RuntimeError(f'{obj}: expected one {stem} {kind}, found {found}')
        index = symbols[found[0]]
        if (not 0 < index < len(sections) or
                sections[index][1:] != (section_type, wanted)):
            raise RuntimeError(f'{obj}: {found[0]} has wrong section type or flags')


FUNCTION_RETAINED = {
    'keep_default', 'keep_static', 'retain_prototype', 'retain_after',
    'used_prototype', 'used_after', 'keep_custom', 'keep_cold', 'jump_fn',
}
FUNCTION_DEAD = {'used_not_retain', 'unused_retain_only', 'unused_retain_data'}


def check_function_object(obj: Path) -> None:
    sections, symbols = elf(obj)
    for name in FUNCTION_RETAINED:
        index = symbols.get(name, 0)
        if not 0 < index < len(sections) or sections[index][1:] != (1, A | X | R):
            raise RuntimeError(f'{obj}: retained function {name} has wrong section: '
                               f'{sections[index] if 0 < index < len(sections) else index}')
        if name == 'keep_custom' and sections[index][0] != '.custom_fn':
            raise RuntimeError(f'{obj}: explicit function section was lost')
        if name not in ('keep_custom', 'keep_cold') and sections[index][0] != f'.text.{name}':
            raise RuntimeError(f'{obj}: {name} must have its own text section')
    index = symbols.get('used_not_retain', 0)
    if not 0 < index < len(sections) or sections[index] != ('.discard_fn', 1, A | X):
        raise RuntimeError(f'{obj}: used without retain must not gain SHF_GNU_RETAIN')
    if any(symbols.get(name, 0) != 0 for name in ('unused_retain_only', 'unused_retain_data')):
        raise RuntimeError(f'{obj}: retain alone unexpectedly prevented frontend DCE at -O2')


def check_function_gc(binary: Path, *, o0: bool = False) -> None:
    _, symbols = elf(binary)
    present = {name for name, index in symbols.items() if index != 0}
    expected = FUNCTION_RETAINED | ({'unused_retain_only', 'unused_retain_data'} if o0 else set())
    missing = expected - present
    unexpected = (FUNCTION_DEAD - expected) & present
    if missing or unexpected:
        raise RuntimeError(f'{binary}: retained functions missing {missing}; '
                           f'unretained functions present {unexpected}')
    if {'helper_a', 'helper_b'} - present:
        raise RuntimeError(f'{binary}: jump-table dependencies were discarded')


def check_local_object_sizes(obj: Path, pointer_bytes: int) -> None:
    """Stack-slot padding must never become ELF static/TLS object size."""
    expected = {
        'c': 1, 'c_zero': 1, 'h': 2, 'i': 4, 'i_zero': 4,
        'f': 4, 'd': 8, 'tls': 4, 'array': 12,
        'aggregate': 5 * pointer_bytes, 'flexible': 16, 'vector': 16,
        'tiny': 2, 'decl_vec': 16, 'p': pointer_bytes, 'q': pointer_bytes,
    }
    found = {}
    for line in run(['readelf', '-sW', obj]).stdout.splitlines():
        fields = line.split()
        if len(fields) >= 8 and fields[3] in ('OBJECT', 'TLS'):
            parts = fields[7].split('.')
            if len(parts) >= 2 and parts[-2] in expected and parts[-1].isdigit():
                name = parts[-2]
                if name in found:
                    raise RuntimeError(f'{obj}: duplicate local symbol {name}')
                found[name] = int(fields[2])
    if found != expected:
        raise RuntimeError(f'{obj}: local object sizes {found}, expected {expected}')
    # A pointer-to-struct's brace initializer must not append pointee padding.
    sections, _ = elf(obj)
    table_indices = [i for i, section in enumerate(sections) if section[0] == '.ptr_table']
    if len(table_indices) != 1:
        raise RuntimeError(f'{obj}: expected one .ptr_table')
    data = obj.read_bytes()
    if data[4] == 2:
        shoff, shentsize = struct.unpack_from('<Q', data, 40)[0], struct.unpack_from('<H', data, 58)[0]
        size = struct.unpack_from('<Q', data, shoff + table_indices[0] * shentsize + 32)[0]
    else:
        shoff, shentsize = struct.unpack_from('<I', data, 32)[0], struct.unpack_from('<H', data, 46)[0]
        size = struct.unpack_from('<I', data, shoff + table_indices[0] * shentsize + 20)[0]
    if size != 2 * pointer_bytes:
        raise RuntimeError(f'{obj}: .ptr_table size {size}, expected {2 * pointer_bytes}')
    for name, wanted_size, wanted_align in [('.tiny_vector', 2, 2),
                                            ('.decl_vector', 16, 16),
                                            ('.global_vector', 16, 16)]:
        indices = [i for i, section in enumerate(sections) if section[0] == name]
        if len(indices) != 1:
            raise RuntimeError(f'{obj}: expected one {name} section')
        pos = shoff + indices[0] * shentsize
        if data[4] == 2:
            sec_size = struct.unpack_from('<Q', data, pos + 32)[0]
            sec_align = struct.unpack_from('<Q', data, pos + 48)[0]
        else:
            sec_size = struct.unpack_from('<I', data, pos + 20)[0]
            sec_align = struct.unpack_from('<I', data, pos + 32)[0]
        if (sec_size, sec_align) != (wanted_size, wanted_align):
            raise RuntimeError(f'{obj}: {name} has size/alignment {sec_size}/{sec_align}, '
                               f'expected {wanted_size}/{wanted_align}')


def check_split(obj: Path, target: str, case: int) -> None:
    sections, symbols = elf(obj)
    if not all(sym in symbols and 0 < symbols[sym] < len(sections) for sym in ('first', 'second')):
        raise RuntimeError(f'{target} conflict {case}: missing defined symbols')
    first, second = (sections[symbols[sym]] for sym in ('first', 'second'))
    expected = (False, True) if case in (0, 5) else (True, False)
    if first[0] != second[0] or (bool(first[2] & R), bool(second[2] & R)) != expected:
        raise RuntimeError(f'{target} conflict {case}: wrong same-name section identities: {first}, {second}')


def check_gc(binary: Path, *, mutation: bool = False) -> None:
    _, symbols = elf(binary)
    present = {name for name, index in symbols.items() if index != 0}
    if mutation:
        if present & (RETAINED | DISCARDED):
            raise RuntimeError(f'{binary}: negative control unexpectedly kept {present & (RETAINED | DISCARDED)}')
    else:
        if RETAINED - present or present & DISCARDED:
            raise RuntimeError(f'{binary}: GC retained missing {RETAINED - present}, '
                               f'discarded present {present & DISCARDED}')


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lccc', default=os.environ.get('CCC', str(ROOT / 'target/fastbuild/lccc')))
    parser.add_argument('--gcc', default=os.environ.get('GCC16', 'gcc-16'))
    parser.add_argument('--as', dest='gas', default=os.environ.get('AS', 'as'))
    parser.add_argument('--ld', default=os.environ.get('LD', 'ld'))
    parser.add_argument('--mold', default=os.environ.get('MOLD', 'mold'))
    parser.add_argument('--aarch64-as', help='optional AArch64 GNU assembler oracle')
    parser.add_argument('--riscv-as', help='optional RISC-V GNU assembler oracle')
    args = parser.parse_args()
    compiler = Path(args.lccc).resolve()
    directory = compiler.parent
    targets = [
        ('x86-64', compiler, [], args.gas, ['--64']),
        ('i686', directory / 'lccc-i686', [], args.gas, ['--32']),
        ('aarch64', directory / 'lccc-arm', [], args.aarch64_as, []),
        ('riscv64', directory / 'lccc-riscv', [], args.riscv_as, ['-march=rv64gc', '-mabi=lp64d']),
        ('riscv32', directory / 'lccc-riscv', ['-march=rv32gc', '-mabi=ilp32d'],
         args.riscv_as, ['-march=rv32gc', '-mabi=ilp32d']),
    ]
    if run([args.gcc, '-dumpfullversion']).stdout.strip() != '16.2.0':
        raise RuntimeError('GCC 16.2.0 is required')
    with tempfile.TemporaryDirectory(prefix='lccc-retain-sections-') as tmp:
        work = Path(tmp)
        source = work / 'input.s'
        source.write_text(ASM)
        objects = {}
        oracle_count = 0
        for name, cc, flags, gas, gasflags in targets:
            obj = work / f'{name}.o'
            run([cc, *flags, '-c', source, '-o', obj])
            check_sections(obj, EXPECTED)
            objects[name] = obj
            if gas:
                reference = work / f'{name}-gas.o'
                run([gas, *gasflags, '-o', reference, source])
                check_sections(reference, EXPECTED)
                oracle_count += 1
        # If same-name variants are not implemented, all paths must fail
        # instead of silently assigning the wrong retain bit to either symbol.
        conflicts = [
            '.section .split,"aw",@progbits\nfirst: .long 1\n.section .split,"awR",@progbits\nsecond: .long 2\n',
            '.section .split,"awR",@progbits\nfirst: .long 1\n.section .split,"aw",@progbits\nsecond: .long 2\n',
            '.section .split,"awR",@progbits\nfirst: .long 1\n.section .split\nsecond: .long 2\n',
            '.section .split,"awR",@progbits\nfirst: .long 1\n.section .split,""\nsecond: .long 2\n',
            '.section .data,"awR",@progbits\nfirst: .long 1\n.data\nsecond: .long 2\n',
            '.section .split,"aw",@progbits\nfirst: .long 1\n.pushsection .split,"awR",@progbits\nsecond: .long 2\n.popsection\n',
        ]
        for name, cc, flags, gas, gasflags in targets:
            for i, text in enumerate(conflicts):
                src, obj = work / f'{name}-conflict-{i}.s', work / f'{name}-conflict-{i}.o'
                src.write_text('.globl first,second\n' + text)
                if gas:
                    reference = work / f'{name}-gas-conflict-{i}.o'
                    run([gas, *gasflags, '-o', reference, src])
                    check_split(reference, f'{name} GAS', i)
                result = run([cc, *flags, '-c', src, '-o', obj], check=False)
                if result.returncode:
                    if not any(x in result.stderr for x in ('same-named retained', 'changed section attributes')):
                        raise RuntimeError(f'{name} conflict {i}: wrong failure: {result.stderr}')
                else:
                    check_split(obj, name, i)
        # C frontend, not just handwritten assembly: both initialized and
        # zero TLS, const TLS, explicit sections, and function-scope statics.
        # RV32 is an assembler-only target here; compile the other four.
        tls_source = ROOT / 'tests/regression/fixtures/retain-c/tls.c'
        tls_reference = work / 'gcc-tls.o'
        run([args.gcc, '-O2', '-fPIC', '-c', tls_source, '-o', tls_reference])
        check_tls_source_object(tls_reference)
        for name, cc, flags, _, _ in targets[:-1]:
            tls_obj = work / f'{name}-tls.o'
            run([cc, *flags, '-O2', '-fPIC', '-c', tls_source, '-o', tls_obj])
            check_tls_source_object(tls_obj)
            if name == 'x86-64':
                # Link the LCCC object with both GCC's GNU linker and LCCC's
                # own linker; section-bit-only tests miss a malformed TLS image.
                for linker_cc in (args.gcc, compiler):
                    exe = work / f'tls-{Path(linker_cc).name}'
                    run([linker_cc, '-Wl,--gc-sections', tls_obj, '-o', exe])
                    run([exe])
        # Verify separate per-thread initialized and zero TLS images. Compile
        # with both frontends and link each with both native linker drivers.
        thread_source = ROOT / 'tests/regression/fixtures/retain-c/tls-thread.c'
        for frontend in (args.gcc, compiler):
            for opt in ('-O0', '-O1', '-O2', '-O3', '-Os'):
                for pic in ('-fPIC', '-fno-pic'):
                    stem = f'thread-{Path(frontend).name}-{opt[1:]}-{pic[2:]}'
                    obj = work / f'{stem}.o'
                    run([frontend, opt, pic, '-pthread', '-c', thread_source, '-o', obj])
                    for linker_cc in (args.gcc, compiler):
                        exe = work / f'{stem}-{Path(linker_cc).name}'
                        run([linker_cc, '-no-pie', '-pthread', '-Wl,--gc-sections', obj, '-o', exe])
                        run([exe])
        # Verify exact ELF sizes, including TLS, vector, flexible-array and
        # pointer-to-struct brace initializers; auto stack padding is not a
        # valid object size and can corrupt custom pointer tables.
        local_source = ROOT / 'tests/regression/fixtures/retain-c/local-size.c'
        for name, cc, flags, pointer_bytes in [
                ('gcc-64', args.gcc, [], 8), ('gcc-32', args.gcc, ['-m32'], 4),
                *[(name, cc, flags, 4 if name == 'i686' else 8)
                  for name, cc, flags, _, _ in targets[:-1]]]:
            obj = work / f'{name}-local-size.o'
            run([cc, *flags, '-O2', '-fPIC', '-c', local_source, '-o', obj])
            check_local_object_sizes(obj, pointer_bytes)
            if name in ('gcc-64', 'x86-64'):
                exe = work / f'{name}-local-size'
                run([args.gcc, obj, '-o', exe])
                run([exe])

        # C function retain differs from `used`: it must root its own code
        # section, including when inherited from earlier or later prototypes.
        # The switch forces an in-function .rodata round trip on each backend.
        function_source = ROOT / 'tests/regression/fixtures/retain-c/functions.c'
        function_reference = work / 'gcc-functions.o'
        run([args.gcc, '-O2', '-ffunction-sections', '-c', function_source, '-o', function_reference])
        check_function_object(function_reference)
        for name, cc, flags, _, _ in targets[:-1]:
            obj = work / f'{name}-functions.o'
            asm = work / f'{name}-functions.s'
            opts = [*flags, '-O2', '-ffunction-sections']
            run([cc, *opts, '-c', function_source, '-o', obj])
            check_function_object(obj)
            run([cc, *opts, '-S', function_source, '-o', asm])
            text = asm.read_text()
            body = text.split('jump_fn:', 1)[1].split('.size jump_fn', 1)[0]
            if '.section .rodata' not in body or '.section .text.jump_fn,"axR"' not in body:
                raise RuntimeError(f'{name}: jump-table section round trip lost the retain flag')
            if name == 'x86-64':
                for linker_cc in (args.gcc, compiler):
                    exe = work / f'functions-{Path(linker_cc).name}'
                    run([linker_cc, '-no-pie', '-Wl,--gc-sections', obj, '-o', exe])
                    run([exe])
                    check_function_gc(exe)
        # Check the GCC-produced object under both native linker drivers too.
        for linker_cc in (args.gcc, compiler):
            exe = work / f'functions-gcc-{Path(linker_cc).name}'
            run([linker_cc, '-no-pie', '-Wl,--gc-sections', function_reference, '-o', exe])
            check_function_gc(exe)
        # -O0 retains unused retain-only definitions; optimized GCC may DCE
        # them. Neither behavior should depend on -ffunction-sections.
        for frontend in (args.gcc, compiler):
            for opt in ('-O0', '-O1', '-O2', '-O3', '-Os'):
                for fs in ('-ffunction-sections', '-fno-function-sections'):
                    obj = work / 'function-matrix.o'
                    exe = work / 'function-matrix'
                    run([frontend, opt, fs, '-c', function_source, '-o', obj])
                    run([args.gcc, '-no-pie', '-Wl,--gc-sections', obj, '-o', exe])
                    check_function_gc(exe, o0=opt == '-O0')

        # A changed attribute/type within the *same* R variant is invalid in
        # GAS, unlike the two R/non-R variants above.
        invalid = [
            ('.section .bad,"aR",@progbits\n.long 1\n.section .bad,"awR",@progbits\n',
             'changed section attributes'),
            ('.section .bad,"awR",@progbits\n.long 1\n.section .bad,"awR",@nobits\n',
             'changed section type'),
            ('.section .bad,"awR",@nobits\n.zero 4\n.section .bad,"awR"\n',
             'changed section attributes'),
        ]
        for name, cc, flags, gas, gasflags in targets:
            for i, (text, diagnostic) in enumerate(invalid):
                src, obj = work / f'{name}-invalid-{i}.s', work / f'{name}-invalid-{i}.o'
                src.write_text(text)
                commands = [[cc, *flags, '-c', src, '-o', obj]]
                if gas:
                    commands.append([gas, *gasflags, '-o', obj, src])
                for command in commands:
                    result = run(command, check=False)
                    if result.returncode == 0 or diagnostic not in result.stderr:
                        raise RuntimeError(f'{name} invalid {i}: expected {diagnostic}: {result.stderr}')
        # Raw object links, no host startup files. GNU ld and mold must honor
        # the flag on both x86 ABIs; the LCCC linker has GC only on x86-64.
        # i686 LCCC --gc-sections is a separate unimplemented linker feature.
        for name, emulation in [('x86-64', 'elf_x86_64'), ('i686', 'elf_i386')]:
            linkers = (args.ld, args.mold, directory / 'lccc-ld') if name == 'x86-64' else (args.ld, args.mold)
            for linker in linkers:
                out = work / f'{name}-{Path(linker).name}'
                run([linker, '-m', emulation, '--gc-sections', '-e', '_start', '-o', out, objects[name]])
                check_gc(out)
        # A partial link must carry R and GNU OSABI into the next link, and
        # must keep R/non-R sections with the same name distinct. Test GNU as
        # input because the integrated assemblers currently refuse that case.
        mixed = work / 'x86-64-gas-conflict-0.o'
        for linker in (args.ld, directory / 'lccc-ld'):
            combined = work / f'partial-{Path(linker).name}.o'
            run([linker, '-m', 'elf_x86_64', '-r', '-o', combined, objects['x86-64'], mixed])
            check_sections(combined, EXPECTED)
            check_split(combined, f'{Path(linker).name} partial link', 0)
            out = work / f'final-{Path(linker).name}'
            run([args.ld, '-m', 'elf_x86_64', '--gc-sections', '-e', '_start', '-o', out, combined])
            check_gc(out)
            _, symbols = elf(out)
            defined = {name for name, index in symbols.items() if index != 0}
            if 'second' not in defined or 'first' in defined:
                raise RuntimeError(f'{linker} -r merged retained and discardable input')
        # A relocation against a discarded COMDAT section must not be
        # silently redirected to an unrelated same-named retain variant.
        group_a, group_b = work / 'group-a.s', work / 'group-b.s'
        group_a.write_text('.section .member,"awG",@progbits,grp,comdat\n.long 71\n')
        group_b.write_text('.section .member,"awGR",@progbits,grp,comdat\n.long 99\n'
                           '.section .refs,"aw",@progbits\n.quad .member\n')
        group_a_obj, group_b_obj = work / 'group-a.o', work / 'group-b.o'
        run([args.gas, '--64', '-o', group_a_obj, group_a])
        run([args.gas, '--64', '-o', group_b_obj, group_b])
        for linker, reason in ((args.ld, 'discarded section'), (directory / 'lccc-ld', 'dropped symbol')):
            result = run([linker, '-m', 'elf_x86_64', '-r', '-o', work / 'bad-group.o',
                          group_a_obj, group_b_obj], check=False)
            if result.returncode == 0 or reason not in result.stderr:
                raise RuntimeError(f'{linker} -r: expected {reason}: {result.stderr}')
        mutant = work / 'mutant.s'
        mutant.write_text(ASM.replace('awTR', 'awT').replace('awR', 'aw').replace('aR', 'a').replace('axR', 'ax'))
        mutant_obj = work / 'mutant.o'
        run([compiler, '-c', mutant, '-o', mutant_obj])
        if mutant_obj.read_bytes()[7] != 0:
            raise RuntimeError('non-retain negative control must remain ELFOSABI_NONE')
        for linker in (args.ld, args.mold, directory / 'lccc-ld'):
            mutant_out = work / f'mutant-{Path(linker).name}'
            run([linker, '-m', 'elf_x86_64', '--gc-sections', '-e', '_start', '-o', mutant_out, mutant_obj])
            check_gc(mutant_out, mutation=True)
        ifunc_src, ifunc_obj = work / 'ifunc.s', work / 'ifunc.o'
        ifunc_src.write_text('.text\n.globl resolver\n.type resolver,@gnu_indirect_function\nresolver: ret\n')
        run([args.gas, '--64', '-o', ifunc_obj, ifunc_src])
        for linker in (args.ld, directory / 'lccc-ld'):
            plain = work / f'plain-{Path(linker).name}.o'
            run([linker, '-m', 'elf_x86_64', '-r', '-o', plain, mutant_obj])
            if plain.read_bytes()[7] != 0:
                raise RuntimeError(f'{linker} -r: plain object must stay ELFOSABI_NONE')
            ifunc = work / f'ifunc-{Path(linker).name}.o'
            run([linker, '-m', 'elf_x86_64', '-r', '-o', ifunc, ifunc_obj])
            if ifunc.read_bytes()[7] != 3:
                raise RuntimeError(f'{linker} -r: GNU IFUNC must retain ELFOSABI_GNU')
        # Also assemble real GCC 16 output, not just a handwritten spelling.
        gcc_source = work / 'gcc.c'
        gcc_source.write_text('''int kept_gcc __attribute__((retain)) = 17;
int dropped_gcc = 19;
__attribute__((retain,noinline)) int kept_fn(void) { return kept_gcc; }
__attribute__((noinline)) int dropped_fn(void) { return dropped_gcc; }
int main(void) { return 0; }
''')
        gcc_asm, gcc_obj, gcc_exe = work / 'gcc.s', work / 'gcc.o', work / 'gcc-exe'
        run([args.gcc, '-O2', '-fdata-sections', '-ffunction-sections', '-S', gcc_source, '-o', gcc_asm])
        run([compiler, '-c', gcc_asm, '-o', gcc_obj])
        check_sections(gcc_obj, {'.data.kept_gcc': (1, A | W | R), '.text.kept_fn': (1, A | X | R)})
        run([args.gcc, '-Wl,--gc-sections', gcc_obj, '-o', gcc_exe])
        _, symbols = elf(gcc_exe)
        defined = {name for name, index in symbols.items() if index != 0}
        if not {'kept_gcc', 'kept_fn'} <= defined or {'dropped_gcc', 'dropped_fn'} & defined:
            raise RuntimeError('GCC assembly: retained or discarded symbols have wrong GC result')
    print(f'PASS: GNU retain sections on {len(targets)} integrated assemblers, '
          f'{oracle_count} GAS oracles, 30 collision checks, GNU/mold x86/i686 GC and LCCC x86-64 GC, '
          'GNU/LCCC partial links, GCC 16 assembly, C TLS/thread runtime, '
          'static object layouts, retained-function GC and section restoration and negative controls')


if __name__ == '__main__':
    try:
        main()
    except (RuntimeError, ValueError, IndexError, struct.error) as error:
        raise SystemExit(f'FAIL: {error}')
