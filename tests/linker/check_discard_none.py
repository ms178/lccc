#!/usr/bin/env python3
"""Kernel --discard-none contract, including local ABS and STT_FILE records.

This is a non-kernel assembly fixture. GNU as is an independent ELF producer;
GNU ld is the oracle. --fatal-warnings must remain enabled. --discard-none is
not a warning-suppression exception, and does not undo --strip-all.
"""
import argparse
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
SOURCE = '''.file "discard-fixture.S"
.text
.globl _start
_start: .byte 0xc3
.local kept_local
.type kept_local,@object
kept_local: .byte 0x42
.size kept_local,1
.local kept_absolute
.set kept_absolute,0x1234
.section .note.GNU-stack,"",@progbits
'''
SCRIPT = '''ENTRY(_start)
PHDRS { text PT_LOAD FLAGS(5); }
SECTIONS { . = 0x400000; .text : { *(.text) } :text }
'''
NAMES = {'kept_local', 'kept_absolute', 'discard-fixture.S'}


def execute(command):
    proc = subprocess.run(list(map(str, command)), text=True, capture_output=True, timeout=30)
    if proc.returncode or proc.stderr:
        raise AssertionError(f'{command}: rc={proc.returncode}\n{proc.stdout}\n{proc.stderr}')
    return proc.stdout


def symbols(path):
    text = execute(['readelf', '-sW', path])
    found = {}
    for line in text.splitlines():
        fields = line.split()
        if len(fields) >= 8 and fields[7] in NAMES:
            name = fields[7]
            if name in found:
                raise AssertionError(f'duplicate symbol {name}: {text}')
            found[name] = (fields[2], fields[3], fields[4], fields[5],
                           fields[6] if fields[6] == 'ABS' else 'SECTION')
    return found


def run(linker):
    checked = 0
    with tempfile.TemporaryDirectory(prefix='lccc-discard-none-') as tmp:
        root = Path(tmp)
        (root/'fixture.s').write_text(SOURCE)
        (root/'fixture.ld').write_text(SCRIPT)
        for bits, emulation in [(64, 'elf_x86_64'), (32, 'elf_i386')]:
            obj = root/'fixture.o'
            execute(['as', f'--{bits}', root/'fixture.s', '-o', obj])
            for script in [[], ['-T', root/'fixture.ld']]:
                for options in [[], ['--discard-none'], ['-s', '--discard-none'], ['--discard-none', '-s']]:
                    outputs = []
                    signatures = []
                    for label, ld in [('bfd', 'ld.bfd'), ('lccc', linker)]:
                        out = root/f'{label}.elf'
                        execute([ld, '-m', emulation, '--fatal-warnings', *script,
                                 *options, '-o', out, obj])
                        signatures.append(symbols(out))
                        raw = root/f'{label}.bin'
                        # Default link scripts have different segment layouts; compare
                        # code there, and the entire flat image under the pinned script.
                        sections = [] if script else ['--only-section=.text']
                        execute(['objcopy', '-O', 'binary', *sections, out, raw])
                        outputs.append(raw.read_bytes())
                    if signatures[0] != signatures[1] or outputs[0] != outputs[1]:
                        raise AssertionError(f'{bits} {script} {options}: symbols {signatures}; bytes equal={outputs[0] == outputs[1]}')
                    if '-s' not in options and set(signatures[1]) != NAMES:
                        raise AssertionError(f'missing input locals: {signatures[1]}')
                    checked += 1
    print(f'--discard-none: PASS ({checked} strict ELF32/64 script/default links against GNU ld)')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--linker', default=str(ROOT/'target/fastbuild/lccc-ld'))
    run(parser.parse_args().linker)
