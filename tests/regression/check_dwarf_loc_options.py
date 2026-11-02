#!/usr/bin/env python3
"""Compare DWARF line-state flags against GNU as, including section switches.

Real emitted rows are decoded from readelf's line program (not just opcode
presence). Test four built-in assemblers and their GNU-linked images. A stale
assembler that discards .loc flags must fail against the explicit oracle.
"""
import argparse
import random
import re
import shutil
import subprocess
import tempfile
from pathlib import Path

SOURCE = '''.file 1 "flags.c"
.text
.globl flags_entry
flags_entry:
.loc 1 10 1 is_stmt 0 prologue_end discriminator 257 isa 3
nop
.loc 1 11 2 basic_block epilogue_begin
nop
.section .text.extra,"ax"
.loc 1 20 3 discriminator 2
nop
.text
.loc 1 12 4 is_stmt 1 isa 0
nop
'''
# line, is_stmt, isa, basic_block, prologue_end, epilogue_begin, discriminator
EXPECTED = [(10, False, 3, False, True, False, 257),
            (11, False, 3, True, False, True, 0),
            (12, True, 0, False, False, False, 0),
            (20, False, 3, False, False, False, 2)]


def run(args):
    p = subprocess.run([str(x) for x in args], text=True, capture_output=True)
    if p.returncode:
        raise RuntimeError(f"command failed: {args}\n{p.stdout}\n{p.stderr}")
    return p.stdout


def rows(path):
    text = run(["readelf", "--debug-dump=rawline", path])
    result = []
    line, stmt, isa, basic, pro, epi, disc = 1, True, 0, False, False, False, 0
    for raw in text.splitlines():
        if 'End of Sequence' in raw:
            line, stmt, isa, basic, pro, epi, disc = 1, True, 0, False, False, False, 0
            continue
        match = re.search(r'Set is_stmt to (\d+)', raw)
        if match: stmt = bool(int(match.group(1)))
        match = re.search(r'Set ISA to (\d+)', raw)
        if match: isa = int(match.group(1))
        match = re.search(r'set Discriminator to (\d+)', raw)
        if match: disc = int(match.group(1))
        if 'Set basic block' in raw: basic = True
        if 'Set prologue_end to true' in raw: pro = True
        if 'Set epilogue_begin to true' in raw: epi = True
        match = re.search(r'(?:Advance Line|Line) by -?\d+ to (\d+)', raw)
        if match: line = int(match.group(1))
        if re.search(r'\bCopy(?: \(view \d+\))?\s*$', raw) or 'Special opcode' in raw:
            result.append((line, stmt, isa, basic, pro, epi, disc))
            basic, pro, epi, disc = False, False, False, 0
    return sorted(result)


def special_opcode_check(work, cc):
    # Both rows at an identical address and opcode-boundary/fallback cases.
    # Compare addresses as well as line numbers; opcode-only tests miss a
    # writer that emits correct lines attached to the wrong instruction.
    src = work / 'special.s'
    src.write_text('''.file 1 "special.c"
.text
.globl flags_entry
flags_entry:
.loc 1 10 1
nop
.loc 1 5 1
.loc 1 13 2
nop
.loc 1 7 2
nop
.loc 1 16 2
''' + 'nop\n' * 17 + '''.loc 1 21 2
''' + 'nop\n' * 17 + '''.loc 1 16 2
nop
.loc 1 1 2
nop
''')
    expected = [(10, 0), (5, 1), (13, 1), (7, 2), (16, 3),
                (21, 20), (16, 37), (1, 38)]
    for who in ('gnu', 'lccc'):
        obj = work / f'special-{who}.o'
        if who == 'gnu': run(['as', '-o', obj, src])
        else: run([cc, '-x', 'assembler', '-c', src, '-o', obj])
        if who == 'lccc':
            raw = run(['readelf', '--debug-dump=rawline', obj])
            if raw.count('Special opcode') < 3:
                raise RuntimeError('x86_64 line program did not compact eligible rows')
        for path in (obj, work / f'special-{who}.elf'):
            if path != obj:
                run(['ld', '-e', 'flags_entry', '-Ttext=0', '-o', path, obj])
            text = run(['readelf', '--debug-dump=decodedline', path])
            found = []
            for raw in text.splitlines():
                fields = raw.split()
                if len(fields) >= 3 and fields[0] == 'special.c' and fields[1].isdigit():
                    found.append((int(fields[1]), int(fields[2], 16)))
            if found != expected:
                raise RuntimeError(f'{who} {path.name}: expected {expected}, got {found}')
    print('PASS x86_64: special opcode boundaries, fallback and row addresses')


def special_opcode_seeded_check(work, cc):
    rng = random.Random(0xD0A4F)
    source = ['.file 1 "special-fuzz.c"', '.text', '.globl flags_entry', 'flags_entry:']
    expected, address, line = [], 0, 5000
    for i in range(1000):
        line += rng.choice([-30, -6, -5, -1, 1, 8, 9, 30])
        expected.append((line, address))
        source.append(f'.loc 1 {line} {i % 4 + 1}')
        advance = rng.choice([0, 0, 1, 1, 2, 16, 17, 18, 36])
        source.extend(['nop'] * advance)
        address += advance
    source.append('nop')  # anchor the last location even if its advance is zero
    src = work / 'special-fuzz.s'
    src.write_text('\n'.join(source) + '\n')
    for who in ('gnu', 'lccc'):
        obj = work / f'special-fuzz-{who}.o'
        if who == 'gnu': run(['as', '-o', obj, src])
        else: run([cc, '-x', 'assembler', '-c', src, '-o', obj])
        image = work / f'special-fuzz-{who}.elf'
        run(['ld', '-e', 'flags_entry', '-Ttext=0', '-o', image, obj])
        for path in (obj, image):
            found = []
            for raw in run(['readelf', '--debug-dump=decodedline', path]).splitlines():
                fields = raw.split()
                if len(fields) >= 3 and fields[0] == 'special-fuzz.c' and fields[1].isdigit():
                    found.append((int(fields[1]), int(fields[2], 16)))
            if found != expected:
                first = next((i for i, (a, b) in enumerate(zip(found, expected)) if a != b),
                             min(len(found), len(expected)))
                raise RuntimeError(f'{who} {path.name}: seeded row mismatch at {first}: '
                                   f'got {found[first:first+1]}, expected {expected[first:first+1]} '
                                   f'({len(found)} vs {len(expected)} rows)')
    print('PASS x86_64: 1000 seeded line/address rows, relocatable and linked')


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--lccc', default='target/fastbuild/lccc')
    ap.add_argument('--i686', default='target/fastbuild/lccc-i686')
    ap.add_argument('--arm', default='target/fastbuild/lccc-arm')
    ap.add_argument('--riscv', default='target/fastbuild/lccc-riscv')
    a = ap.parse_args()
    legs = [('x86_64', a.lccc, 'as', 'ld', []),
            ('i686', a.i686, 'as', 'ld', ['--32']),
            ('aarch64', a.arm, 'aarch64-linux-gnu-as', 'aarch64-linux-gnu-ld', []),
            ('riscv64', a.riscv, 'riscv64-linux-gnu-as', 'riscv64-linux-gnu-ld', [])]
    with tempfile.TemporaryDirectory(prefix='lccc-dwarf-loc-') as d:
        work = Path(d)
        src = work / 'loc.s'
        src.write_text(SOURCE)
        special_opcode_check(work, str(Path(a.lccc).resolve()))
        special_opcode_seeded_check(work, str(Path(a.lccc).resolve()))
        for name, cc, gas, ld, gas_flags in legs:
            if not shutil.which(gas) or not shutil.which(ld):
                if name in ('x86_64', 'i686'):
                    raise RuntimeError(f'{name}: missing GNU oracle {gas}/{ld}')
                print(f'SKIP {name}: GNU oracle not installed')
                continue
            cc = str(Path(cc).resolve())
            outputs = []
            for who in ('gnu', 'lccc'):
                obj = work / f'{name}-{who}.o'
                if who == 'gnu': run([gas, *gas_flags, '-o', obj, src])
                else: run([cc, '-x', 'assembler', '-c', src, '-o', obj])
                outputs.append(obj)
                got = rows(obj)
                if got != EXPECTED:
                    raise RuntimeError(f'{name} {who} object: expected {EXPECTED}, got {got}')
                exe = work / f'{name}-{who}.elf'
                ld_flags = ['-m', 'elf_i386'] if name == 'i686' else []
                run([ld, *ld_flags, '-e', 'flags_entry', '-o', exe, obj])
                got = rows(exe)
                if got != EXPECTED:
                    raise RuntimeError(f'{name} {who} linked: expected {EXPECTED}, got {got}')
            print(f'PASS {name}: GNU/LCCC .loc state, relocatable and linked')
    print('DWARF .loc options: PASS')


if __name__ == '__main__':
    try:
        main()
    except (OSError, RuntimeError) as error:
        raise SystemExit(f'FAIL DWARF .loc options: {error}')
