"""Same-object ELF symbol-selection tests for builtin, -T, -r and DSO links."""

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
from dataclasses import dataclass


@dataclass(frozen=True)
class Target:
    name: str
    emulation: str
    triple: str
    word: str
    ret: str
    exit_asm: str


TARGETS = (
    Target('x86_64', 'elf_x86_64', 'x86_64-linux-gnu', '.quad', 'ret',
           'mov $60,%eax; xor %edi,%edi; syscall'),
    Target('i386', 'elf_i386', 'i686-linux-gnu', '.long', 'ret',
           'mov $1,%eax; xor %ebx,%ebx; int $0x80'),
    Target('aarch64', 'aarch64linux', 'aarch64-linux-gnu', '.quad', 'ret',
           'mov x8,#93; mov x0,#0; svc #0'),
    Target('riscv64', 'elf64lriscv', 'riscv64-linux-gnu', '.quad', 'ret',
           'li a7,93; li a0,0; ecall'),
)

SCRIPT = '''ENTRY(_start)
PHDRS { code PT_LOAD FLAGS(5); data PT_LOAD FLAGS(6); }
SECTIONS {
  . = 0x400000; .text : { *(.text .text.*) } :code
  . = ALIGN(0x1000); .data : { *(.data) } :data
  .bss : { *(.bss) *(COMMON) } :data
  DISCARD
}
'''


@dataclass(frozen=True)
class Symbol:
    value: int
    size: int
    binding: str
    visibility: str
    index: str


class Fixture:
    def __init__(self, target, directory, assembler):
        self.target = target
        self.root = Path(directory)
        self.as_ = assembler
        self.cases = []

    def run(self, *cmd):
        return subprocess.run(cmd, cwd=self.root, capture_output=True, text=True,
                              timeout=20, env={**os.environ, 'LC_ALL': 'C'})

    def checked(self, *cmd):
        proc = self.run(*cmd)
        if proc.returncode:
            raise RuntimeError(f'{proc.args}: rc={proc.returncode}: {proc.stderr.strip()}')
        return proc.stdout

    def assemble(self, stem, asm):
        # GNU AArch64 as treats @ as a line comment; % works in GAS and LLVM.
        asm += '\n.section .note.GNU-stack,"",@progbits\n'
        if self.target.name == 'aarch64':
            asm = asm.replace('@', '%')
        (self.root / f'{stem}.s').write_text(asm)
        self.checked(*self.as_, '-o', f'{stem}.o', f'{stem}.s')

    def symbols(self, out, name, dynamic=False):
        txt = self.checked('readelf', '--dyn-syms' if dynamic else '--syms', '--wide', out)
        if not dynamic and "Symbol table '.symtab'" in txt:
            txt = txt[txt.index("Symbol table '.symtab'"):]
        result = []
        for line in txt.splitlines():
            fields = line.split()
            if (len(fields) > 7 and re.fullmatch(r'\d+:', fields[0])
                    and fields[7].split('@')[0] == name):
                result.append(Symbol(int(fields[1], 16), int(fields[2]), *fields[4:7]))
        return result

    def sym(self, out, name='pick'):
        symbols = self.symbols(out, name)
        assert len(symbols) == 1, f'{out}: expected one {name}, got {symbols}'
        return symbols[0]

    def check(self, slug, opts, inputs, predicate=None, diagnostic=None, bfd=True):
        self.cases.append((slug, opts, inputs, predicate, diagnostic, bfd))

    def execute(self, linker, bfd=None):
        report = []
        for who, engine in [('lccc', linker), ('bfd', bfd)]:
            if engine is None:
                continue
            for slug, opts, inputs, predicate, diagnostic, bfd_ok in self.cases:
                if who == 'bfd' and not bfd_ok:
                    continue
                path = f'{who}-{slug}.elf'
                cmd = [engine, '-m', self.target.emulation, '-o', path, *opts,
                       *(name if name.endswith('.a') else f'{name}.o' for name in inputs)]
                proc = self.run(*cmd)
                error = ''
                try:
                    if diagnostic:
                        assert proc.returncode == 1 and diagnostic in proc.stderr, (
                            f'expected {diagnostic!r}: rc={proc.returncode}, {proc.stderr.strip()}')
                        assert not (self.root / path).exists(), 'failed link left an output'
                    else:
                        assert not proc.returncode, f'rc={proc.returncode}: {proc.stderr.strip()}'
                        assert not proc.stderr.strip(), proc.stderr.strip()
                        if predicate:
                            predicate(path)
                except (AssertionError, RuntimeError) as e:
                    error = str(e)
                report.append({'engine': who, 'target': self.target.name, 'case': slug,
                               'status': 'FAIL' if error else 'PASS', 'detail': error})
        return report


def cases(f):
    w = f.target.word
    f.assemble('start', f'.text\n.globl _start\n_start: {f.target.exit_asm}')
    for size, align in [(8, 128), (64, 8)]:
        f.assemble(f'c{size}', f'.comm pick,{size},{align}')
    for binding, directive, value in [('strong', '.globl', 3), ('strong2', '.globl', 7),
                                      ('weak', '.weak', 5)]:
        f.assemble(binding, f'.data\n{directive} pick\n.type pick,@object\n'
                   f'pick: {w} {value}\n.size pick,.-pick')
    f.assemble('strong64', f'.data\n.balign 8\n.globl pick\n.type pick,@object\n'
               f'pick: {w} 3\n.zero {64 - (4 if w == ".long" else 8)}\n.size pick,.-pick')
    for value in (3, 7):
        f.assemble(f'abs{value}', f'.globl pick\n.set pick,{value}')
    f.assemble('ref', f'.data\n{w} pick')
    f.assemble('weakref', f'.data\n.weak pick\n{w} pick')
    f.assemble('hiddenref', '.hidden pick\n' + (f'.data\n{w} pick'))
    f.checked('ar', 'rcs', 'libpick.a', 'strong2.o')

    def symbol(out, *, size=None, binding=None, align=None, index=None, value=None,
               visibility=None, name='pick'):
        s = f.sym(out, name)
        fields = (('size', size), ('binding', binding), ('index', index),
                  ('value', value), ('visibility', visibility))
        for field, expected in fields:
            if expected is not None:
                assert getattr(s, field) == expected, f'{field}: expected {expected}, got {s}'
        if align:
            assert s.value % align == 0, f'{s}: expected alignment {align}'

    def contract(**kw):
        return lambda out: symbol(out, **kw)

    def absent(name, dynamic=False):
        def check(out):
            s = f.symbols(out, name, dynamic)
            assert not s, f'unexpected {name}: {s}'
        return check

    def section_size(out, name):
        txt = f.checked('readelf', '-SW', out)
        m = re.search(r'\]\s+' + re.escape(name) + r'\s+\w+\s+\w+\s+\w+\s+(\w+)', txt)
        return int(m[1], 16) if m else 0

    def no_bss(out):
        size = section_size(out, '.bss')
        assert size == 0, f'superseded COMMON left {size} bytes of .bss'

    def placed_common(out):
        symbol(out, size=64, align=128, binding='GLOBAL')
        s = f.sym(out)
        assert s.value and s.index.isdecimal(), f'COMMON was not placed: {s}'
        header = re.search(r'\[\s*(\d+)\]\s+\.bss\s+NOBITS\b', f.checked('readelf', '-SW', out))
        assert header and s.index == header[1], f'{s}: expected a .bss definition'
        assert section_size(out, '.bss') == 64, 'expected exactly one 64-byte allocation'

    def text_size(n):
        def check(out):
            txt = f.checked('readelf', '-SW', out)
            m = re.search(r'\]\s+\.text(?:\.group)?\s+PROGBITS\s+\w+\s+\w+\s+(\w+)', txt)
            assert m and int(m[1], 16) == n, f'expected {n} bytes in text: {m[1] if m else None}'
        return check

    dup = 'multiple definition'
    # GNU accepts equal absolute constants but rejects different constants
    # and section definitions even when their relative offsets are equal.
    if f.target.name != 'i386':
        start = ['start']
        for mode, opts in [('exe', ['-static']), ('so', ['-shared'])]:
            inputs = start if mode == 'exe' else []
            def check(label, obj, expect=None, err=None, flags=(), bfd=True):
                f.check(f'{mode}-{label}', [*opts, *flags], [*inputs, *obj], expect, err, bfd)
            check('duplicate', ['strong', 'strong2'], err=dup)
            check('permit-first-absolute', ['abs3', 'abs7'], contract(value=3, index='ABS'),
                  flags=('--fatal-warnings', '-z', 'muldefs'))
            check('different-absolute', ['abs3', 'abs7'], err=dup)
            check('equal-absolute', ['abs3', 'abs3'], contract(value=3, index='ABS'))
            check('common-max-first', ['c64', 'c8'], placed_common)
            check('common-max-last', ['c8', 'c64'], placed_common)
            check('common-over-weak', ['weak', 'c64'], contract(size=64, binding='GLOBAL'))
            check('weak-after-common', ['c64', 'weak'], contract(size=64, binding='GLOBAL'))
            check('strong-over-common', ['c64', 'strong64'], no_bss)
            check('no-dead-common-allocation', ['strong64', 'c64'], no_bss)

    if f.target.name in ('x86_64', 'i386', 'riscv64', 'aarch64'):
        first = '.section .text.group,"axG",@progbits,sig,comdat\n'
        f.assemble('comdat', first + '.globl selected\nselected: ret\n')
        f.assemble('comdat-also', first + '.globl selected\nselected: nop; ret\n')
        f.assemble('comdat-ghost', first + '.globl ghost\nghost: nop; ret\n')
        if f.target.name != 'i386':
            f.assemble('comdat-hidden', first + '.globl selected\n.hidden selected\nselected: ret')
            f.assemble('comdat-local-ref', first + '.Lretired: ret\n.data\n' + w + ' .Lretired')
            f.assemble('comdat-global-ref', first + '.globl retired\nretired: ret\n.data\n' + w + ' retired')
            selected = ['comdat', 'comdat-hidden']
            f.check('exe-comdat-merged-visibility', ['-static'], ['start', *selected],
                    lambda out: symbol(out, visibility='HIDDEN', name='selected'))
            f.check('so-comdat-merged-visibility', ['-shared'], selected, absent('selected', True))
            for mode, opts, base in [('exe', ['-static'], ['start']), ('so', ['-shared'], [])]:
                f.check(f'{mode}-live-local-in-loser', opts, [*base, 'comdat', 'comdat-local-ref'],
                        diagnostic='discarded section')
            f.check('exe-live-global-in-loser', ['-static'], ['start', 'comdat', 'comdat-global-ref'],
                    diagnostic='retired')
            def imported(out):
                s = f.symbols(out, 'retired', True)
                assert len(s) == 1 and s[0].binding == 'GLOBAL' and s[0].index == 'UND', s
                assert re.search(r'\bretired\b', f.checked('readelf', '-rW', out)), 'missing dynamic use'
            # Without an explicit undefined row in another object, BFD's
            # shared link can emit RELATIVE+0. LCCC keeps the live demand.
            f.check('so-live-global-in-loser', ['-shared'], ['comdat', 'comdat-global-ref'],
                    imported, bfd=False)
            f.assemble('comdat-other-demand', '.data\n' + w + ' retired')
            f.check('so-ordinary-undefined-demand', ['-shared'],
                    ['comdat', 'comdat-other-demand'], imported)
            if f.target.name in ('x86_64', 'aarch64'):
                f.check('exe-gc-dead-local-in-loser', ['-static', '--gc-sections'],
                        ['start', 'comdat', 'comdat-local-ref'])
        if f.target.name in ('aarch64', 'riscv64'):
            f.check('exe-comdat-collision', ['-static'], ['start', 'comdat', 'comdat-also'])
            if f.target.name == 'aarch64':
                f.check('exe-comdat-size', ['-static'], ['start', 'comdat', 'comdat-also'], text_size(16))
            f.check('so-comdat-ghost', ['-shared'], ['comdat', 'comdat-ghost'], absent('ghost', True))

    if f.target.name in ('x86_64', 'i386'):
        f.assemble('orphan-weak', '.section .drop,"aw"\n.weak pick\npick: .long 7')
        f.assemble('orphan-dead-only-ref', '.section .drop,"a"\n' + w + ' missing')
        f.assemble('start-data', f'.text\n.globl _start\n_start: {f.target.exit_asm}\n.data\n{w} 0')
        (f.root / 'place.ld').write_text(SCRIPT.replace('DISCARD', ''))
        (f.root / 'drop.ld').write_text(SCRIPT.replace('DISCARD', '/DISCARD/ : { *(.drop) }'))
        opts = ['-T', 'place.ld']
        base = ['start-data']
        f.check('script-duplicate', opts, base + ['strong', 'strong2'], diagnostic=dup)
        f.check('script-duplicate-gc', opts + ['--gc-sections'],
                base + ['strong', 'strong2'], diagnostic=dup)
        f.check('script-repeat-operand', opts, base + ['strong', 'strong'], diagnostic=dup)
        f.check('script-archive-duplicate', opts + ['--whole-archive'],
                base + ['strong', 'libpick.a'], diagnostic=dup)
        f.check('script-opt-in', opts + ['--fatal-warnings', '--allow-multiple-definition'],
                base + ['abs3', 'abs7'], contract(index='ABS', value=3))
        f.check('script-equal-absolute', opts, base + ['abs3', 'abs3'],
                contract(index='ABS', value=3))
        f.check('script-comdat', opts, base + ['comdat', 'comdat-also', 'comdat-ghost'],
                absent('ghost'))
        f.check('script-weak-discard', opts + ['--orphan-handling=discard'],
                base + ['orphan-weak', 'weakref'], diagnostic='pick')
        f.check('script-weak-without-definition', opts, base + ['weakref'])
        f.check('script-discard-only-demand', ['-T', 'drop.ld'],
                base + ['orphan-dead-only-ref'])
        f.assemble('exclude', '.section .omit,"awe",@progbits\n.globl omitted\nomitted: .quad 5')
        f.check('script-shf-exclude', opts, base + ['exclude'], absent('omitted'))
        f.assemble('keep-target', '.section .data.indirect,"aw"\n.globl retained\nretained: .quad 1')
        f.assemble('dead-root', '.section .drop,"awR",@progbits\n' + w + ' retained')
        def retained(out):
            assert f.sym(out, 'retained').index not in ('UND', 'ABS')
        f.check('script-gc-retained-edge', ['-T', 'drop.ld', '--gc-sections'],
                base + ['dead-root', 'keep-target'], retained)

    if f.target.name == 'x86_64':
        f.assemble('unique', '.data\n.globl pick\n.type pick,@gnu_unique_object\npick: .quad 3')
        for mode, opts, base in [('exe', ['-static'], ['start']), ('r', ['-r'], [])]:
            f.check(f'{mode}-unique-duplicate', opts, base + ['strong', 'unique'], diagnostic=dup)
        f.check('r-repeat-operand', ['-r'], ['strong', 'strong'], diagnostic=dup)
        f.check('r-common-max-first', ['-r'], ['c64', 'c8'], contract(value=128, size=64, index='COM'))
        f.check('r-common-max-last', ['-r'], ['c8', 'c64'], contract(value=128, size=64, index='COM'))
        f.check('r-common-over-weak', ['-r'], ['weak', 'c64'], contract(size=64, index='COM'))
        f.check('r-upgrade-strong-reference', ['-r'], ['weakref', 'ref'], contract(binding='GLOBAL', index='UND'))
        f.check('r-hidden-reference', ['-r'], ['strong', 'hiddenref'], contract(visibility='HIDDEN'))
        f.check('r-losing-unique-name', ['-r'], ['comdat', 'comdat-ghost'], absent('ghost'))
        f.check('r-comdat-merged-visibility', ['-r'], ['comdat', 'comdat-hidden'],
                contract(visibility='HIDDEN', name='selected'))
        f.check('r-live-local-in-loser', ['-r'], ['comdat', 'comdat-local-ref'],
                diagnostic='discarded section')
        f.check('r-invalid-muldefs', ['-r', '--fatal-warnings', '-zmuldefs=typo'],
                ['strong'], diagnostic='-z muldefs=typo ignored', bfd=False)
        f.check('r-muldefs', ['-r', '--fatal-warnings', '-zmuldefs'], ['abs3', 'abs7'],
                contract(index='ABS', value=3))
        f.check('exe-losing-unique-name', ['-static'], ['start', 'comdat', 'comdat-ghost'], absent('ghost'))
        f.check('so-losing-unique-name', ['-shared'], ['comdat', 'comdat-ghost'], absent('ghost', True))
        f.check('so-comdat-size', ['-shared'], ['comdat', 'comdat-also'], text_size(1))
        versioned = '''SECTIONS {
            . = SIZEOF_HEADERS;
            .text : { *(.text) }
            .data : { *(.data) }
            .dynamic : { *(.dynamic) }
            .dynsym : { *(.dynsym) }
            .dynstr : { *(.dynstr) }
            .gnu.version : { *(.gnu.version) }
            .gnu.version_d : { *(.gnu.version_d) }
            /DISCARD/ : { *(.drop) }
        }
        VERSION { TEST_1 { global: pick; local: *; }; }
        '''
        (f.root / 'versioned.ld').write_text(versioned)
        opts = ['-shared', '-T', 'versioned.ld', '--no-warn-rwx-segments']
        # BFD can retain a zero-valued ABS export here. LCCC deliberately
        # refuses to export a removed section as a definition.
        f.check('script-dso-discard-export', opts, ['orphan-weak', 'start-data'],
                absent('pick', True), bfd=False)
        f.check('script-dso-hidden-reference', opts, ['strong', 'hiddenref'], absent('pick', True))


def suite(linker, names, require_oracles=True):
    results = []
    for target in TARGETS:
        if target.name not in names:
            continue
        with tempfile.TemporaryDirectory(prefix=f'lccc-symbols-{target.name}-') as tmp:
            if target.name in ('x86_64', 'i386'):
                bfd = shutil.which('ld.bfd') or shutil.which('ld')
                assembler = [os.environ.get('LCCC_TEST_ASSEMBLER', 'as'),
                             '--32' if target.name == 'i386' else '--64']
            else:
                bfd = shutil.which(target.triple + '-ld.bfd') or shutil.which(target.triple + '-ld')
                as_ = shutil.which(target.triple + '-as')
                if as_:
                    assembler = [as_]
                else:
                    clang = shutil.which('clang-23') or shutil.which('clang')
                    if not clang:
                        raise RuntimeError(f'{target.name}: no cross GNU as or Clang')
                    assembler = [clang, f'--target={target.triple}', '-c', '-x', 'assembler']
            if not bfd and require_oracles:
                raise RuntimeError(f'{target.name}: GNU ld oracle missing')
            f = Fixture(target, tmp, assembler)
            cases(f)
            results.extend(f.execute(linker, bfd))
    return results


def run(args, _oracles, Result):
    name = 'elf_symbol_contract'
    linker = str(Path(args.lccc).resolve().with_name('lccc-ld'))
    try:
        native = ('x86_64', 'i386')
        cross = ('aarch64', 'riscv64') if os.environ.get('LCCC_REQUIRE_SYMBOL_CROSS') == '1' else ()
        rows = suite(linker, native + cross)
    except (OSError, RuntimeError, subprocess.TimeoutExpired) as e:
        return Result(name, 'FAIL', str(e))
    failures = [f"{r['engine']}/{r['target']}/{r['case']}: {r['detail']}"
                for r in rows if r['status'] == 'FAIL']
    return Result(name, 'FAIL' if failures else 'PASS',
                  '\n'.join(failures) if failures else f'{len(rows)} ELF symbol checks')


def vendor_matrix(linker, compilers):
    """Native end-to-end selection across independently generated ET_REL inputs."""
    source = {
        'entry': '''extern volatile long pick;
void _start(void) {
    long status = (pick != 0) | ((unsigned long)&pick % 128 != 0);
    __asm__ volatile("syscall" : : "a"(60L), "D"(status) : "rcx", "r11", "memory");
    __builtin_unreachable();
}
''',
        'weak': '__attribute__((weak)) volatile long pick = 17;\n',
        'common': '_Alignas(128) volatile long pick;\n',
    }
    bfd = shutil.which('ld.bfd') or shutil.which('ld')
    if not bfd:
        raise RuntimeError('native vendor matrix requires GNU ld')
    with tempfile.TemporaryDirectory(prefix='lccc-symbols-vendor-') as tmp:
        f = Fixture(TARGETS[0], tmp, ())
        for name, compiler in compilers:
            version = f.checked(compiler, '--version').splitlines()[0]
            print(f'{name}: {version}')
            for stem, code in source.items():
                (f.root / f'{stem}.c').write_text(code)
                f.checked(compiler, '-std=gnu11', '-O2', '-g', '-Wall', '-Wextra', '-Werror',
                          '-ffreestanding', '-fno-stack-protector', '-fno-pic', '-fno-pie',
                          '-fcommon', '-c', f'{stem}.c', '-o', f'{stem}.o')
            for engine in (linker, bfd):
                for order in (('weak.o', 'common.o'), ('common.o', 'weak.o')):
                    f.checked(engine, '-m', 'elf_x86_64', '-static', '--fatal-warnings',
                              '-o', 'probe.elf', 'entry.o', *order)
                    f.checked('./probe.elf')
            print(f'{name}: 4/4 selection and execution checks')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ld', type=lambda path: str(Path(path).resolve()), required=True)
    parser.add_argument('--target', action='append', choices=[t.name for t in TARGETS])
    parser.add_argument('--allow-missing-cross-oracle', action='store_true')
    parser.add_argument('--json', type=Path)
    parser.add_argument('--vendor-cc', metavar='NAME=PROGRAM', action='append')
    args = parser.parse_args()
    if args.vendor_cc:
        compilers = [cc.split('=', 1) for cc in args.vendor_cc]
        if any(len(cc) != 2 or not all(cc) for cc in compilers):
            parser.error('--vendor-cc requires NAME=PROGRAM')
        vendor_matrix(args.ld, compilers)
        if not args.target:
            return False
    rows = suite(args.ld, args.target or ('x86_64', 'i386'), not args.allow_missing_cross_oracle)
    if args.json:
        args.json.write_text(json.dumps(rows, indent=2) + '\n')
    failed = [row for row in rows if row['status'] != 'PASS']
    for row in failed:
        print(f"{row['engine']}/{row['target']}/{row['case']}: {row['detail']}")
    print(f'ELF symbol contracts: {len(rows) - len(failed)}/{len(rows)} pass')
    return bool(failed)


if __name__ == '__main__':
    raise SystemExit(main())
