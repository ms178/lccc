#!/usr/bin/env python3
"""Script segment permissions must reflect actual storage, not default RWX.

Both ELF classes, automatic/PHDRS-inferred/explicit permissions, empty input
markers (including merged ones), executable/data/rodata/NOBITS and reserved
storage are checked against GNU ld. This deliberately discards assembler notes
so GNU as versions cannot change the permission contract being tested.
"""
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]


def command(args):
    return subprocess.run(list(map(str, args)), capture_output=True, text=True, timeout=30)


def permissions(path):
    data = path.read_bytes()
    wide = data[4] == 2
    offset = struct.unpack_from('<Q' if wide else '<I', data, 32 if wide else 28)[0]
    size, count = struct.unpack_from('<HH', data, 54 if wide else 42)
    return [struct.unpack_from('<I', data, offset+i*size+(4 if wide else 24))[0]
            for i in range(count) if struct.unpack_from('<I', data, offset+i*size)[0] == 1]


def run(ld):
    assembler, oracle = shutil.which('as'), shutil.which('ld.bfd')
    assert assembler and oracle
    measured = 0
    with tempfile.TemporaryDirectory(prefix='script-permissions-') as directory:
        work = Path(directory)
        for bits in (32, 64):
            for name, source, expected in (
                ('text-empty-data', '.text\n.globl _start\n_start: ret\n', 5),
                ('rodata', '.section .ro,"a"\n.globl _start\n_start: .byte 1\n', 4),
                ('data', '.data\n.globl _start\n_start: .byte 1\n', 6),
                ('bss', '.bss\n.globl _start\n_start: .space 16\n', 6),
                ('writable-code', '.section .wx,"awx"\n.globl _start\n_start: ret\n', 7),
                ('merged-empty', '.text\n.globl _start\n_start: ret\n', 7),
                ('merged-text', '.text\n.globl _start\n_start: ret\n', 5),
                ('reserved', '.data\n.globl _start\n_start:\n', 6),
            ):
                asm, obj, script = work/'a.s', work/'a.o', work/'a.ld'
                asm.write_text(source+'.section .note.GNU-stack,"",@progbits\n')
                built = command([assembler, f'--{bits}', '-o', obj, asm])
                assert built.returncode == 0, built.stderr
                for mode in ('automatic', 'inferred', 'explicit-0', 'explicit-6', 'explicit-7'):
                    phdr = '' if mode == 'automatic' else 'PHDRS { image PT_LOAD '+(f'FLAGS({mode.split("-")[1]})' if mode.startswith('explicit-') else '')+'; }'
                    tag = '' if mode == 'automatic' else ':image'
                    if name in ('merged-empty', 'merged-text'):
                        out_name = '.text' if name == 'merged-text' else '.out'
                        body = f'{out_name} : {{ *(.text .data .bss) }} {tag}'
                    elif name == 'reserved':
                        body = f'.out : {{ *(.data) . = . + 16; }} {tag}'
                    else:
                        body = ' '.join(f'{s} : {{ *({s}) }} {tag}' for s in ('.text', '.ro', '.data', '.bss', '.wx'))
                    script.write_text(f'ENTRY(_start) {phdr} SECTIONS {{ . = 0x10000; {body} /DISCARD/ : {{ *(.note.*) }} }}')
                    wanted = int(mode.split('-')[1]) if mode.startswith('explicit-') else expected
                    for linker in (ld, oracle):
                        output = work/'output'
                        result = command([linker, '-m', 'elf_i386' if bits == 32 else 'elf_x86_64', '-T', script, '-o', output, obj])
                        assert result.returncode == 0, (bits, name, mode, result.stderr)
                        assert permissions(output) == [wanted], (linker, bits, name, mode, permissions(output), wanted)
                        assert ('LOAD segment with RWX' in result.stderr) == (wanted == 7), result.stderr
                        result = command([linker, '-m', 'elf_i386' if bits == 32 else 'elf_x86_64', '--fatal-warnings', '-T', script, '-o', output, obj])
                        assert bool(result.returncode) == (wanted == 7), (name, mode, result.stderr)
                    measured += 1
    print(f'script segment permissions: PASS ({measured} ELF32/64 GNU-oracle cases)')


if __name__ == '__main__':
    run(ROOT/'target/fastbuild/lccc-ld')
