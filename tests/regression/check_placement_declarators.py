#!/usr/bin/env python3
"""GP-03: declaration-specifier versus individual object attributes.

Use GCC 16 as the x86 oracle; all LCCC targets must satisfy the same IR-level
contract. Check symbol section types/flags, common-vs-defined symbols and GC,
not object byte identity (section order and local symbol names may differ).
"""
import argparse
import subprocess
import tempfile
from pathlib import Path

from check_gnu_retain_sections import elf, R

ROOT = Path(__file__).resolve().parents[2]
SOURCE = r'''
__attribute__((noinit)) int na, nb;
int nc __attribute__((noinit)), nd;
__attribute__((persistent)) int pa=1, pb=2;
int pc __attribute__((persistent))=3, pd=4;
__attribute__((nocommon)) int ca, cb;
int cc __attribute__((nocommon)), cd;
__attribute__((retain)) int ra=11, rb=12;
int rc __attribute__((retain))=13, rd=14;
int first, second __attribute__((noinit)), third;
int pfirst=1, psecond __attribute__((persistent))=2, pthird=3;
int cfirst, csecond __attribute__((nocommon)), cthird;
int rfirst=1, rsecond __attribute__((retain))=2, rthird=3;
void _start(void) { }
'''
EXPECTED = {
    'na': ('.noinit', 8, False), 'nb': ('.noinit', 8, False),
    'nc': ('.noinit', 8, False), 'nd': ('COMMON', None, False),
    'pa': ('.persistent', 1, False), 'pb': ('.persistent', 1, False),
    'pc': ('.persistent', 1, False), 'pd': ('.data', 1, False),
    'ca': ('.bss', 8, False), 'cb': ('.bss', 8, False),
    'cc': ('.bss', 8, False), 'cd': ('COMMON', None, False),
    'ra': ('.data.ra', 1, True), 'rb': ('.data.rb', 1, True),
    'rc': ('.data.rc', 1, True), 'rd': ('.data', 1, False),
    'first': ('COMMON', None, False), 'second': ('.noinit', 8, False),
    'third': ('COMMON', None, False), 'pfirst': ('.data', 1, False),
    'psecond': ('.persistent', 1, False), 'pthird': ('.data', 1, False),
    'cfirst': ('COMMON', None, False), 'csecond': ('.bss', 8, False),
    'cthird': ('COMMON', None, False), 'rfirst': ('.data', 1, False),
    'rsecond': ('.data.rsecond', 1, True), 'rthird': ('.data', 1, False),
}


def run(argv):
    proc = subprocess.run([str(v) for v in argv], cwd=ROOT, capture_output=True, text=True, timeout=30)
    if proc.returncode or proc.stderr:
        raise RuntimeError(f'{argv!r}: rc={proc.returncode}, stderr={proc.stderr!r}')
    return proc


def check(obj):
    sections, symbols = elf(obj)
    for name, (section, kind, retained) in EXPECTED.items():
        index = symbols.get(name)
        if section == 'COMMON':
            got = ('COMMON', None, False) if index == 0xfff2 else ('not COMMON', index, False)
        elif index is None or not 0 < index < len(sections):
            got = ('missing', index, False)
        else:
            secname, sectype, flags = sections[index]
            got = (secname, sectype, bool(flags & R))
        if got != (section, kind, retained):
            raise RuntimeError(f'{obj}: {name}: expected {(section, kind, retained)!r}, got {got!r}')


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--lccc', default='target/fastbuild/lccc')
    ap.add_argument('--gcc', default='gcc-16')
    args = ap.parse_args()
    directory = Path(args.lccc).resolve().parent
    with tempfile.TemporaryDirectory(prefix='lccc-placement-') as temp:
        td = Path(temp)
        source = td / 'placement.c'
        source.write_text(SOURCE, encoding='utf-8')
        compilers = [
            ('gcc-m64', args.gcc, ['-m64']), ('gcc-m32', args.gcc, ['-m32']),
            ('lccc-m64', args.lccc, ['-m64']), ('lccc-m32', args.lccc, ['-m32']),
            ('lccc-arm', directory / 'lccc-arm', []),
            ('lccc-riscv', directory / 'lccc-riscv', []),
        ]
        for label, compiler, flags in compilers:
            obj = td / (label + '.o')
            run([compiler, *flags, '-fcommon', '-c', source, '-o', obj])
            check(obj)
            print(f'{label}: symbol placement PASS ({len(EXPECTED)} objects)')
            if label in ('gcc-m64', 'lccc-m64'):
                exe = td / (label + '.elf')
                run(['ld', '--gc-sections', '-e', '_start', obj, '-o', exe])
                _, names = elf(exe)
                for name in ('ra', 'rb', 'rc', 'rsecond'):
                    if name not in names:
                        raise RuntimeError(f'{label}: retained {name} was collected')
                if 'rd' in names:
                    raise RuntimeError(f'{label}: unretained rd survived section GC')
                print(f'{label}: retain GC positive and negative controls PASS')
    print('object placement declarators: PASS')


if __name__ == '__main__':
    main()
