"""ELF32 symbol selection using identical relocatable inputs for each linker."""

import os
from pathlib import Path
import shutil
import tempfile


STACK = '\n.section .note.GNU-stack,"",@progbits\n'
EXIT = 'mov $1,%eax\nint $0x80\n'
START = '.text\n.globl _start\n_start:\n'


def function(value, binding='.globl', section='.text'):
    return f'{section}\n{binding} pick\n.type pick,@function\npick: mov ${value},%eax\nret\n'


def data(value, binding='.globl', kind='object'):
    return (f'.data\n.p2align 2\n{binding} pick\n.type pick,@{kind}\npick: .long {value}\n'
            '.size pick,.-pick\n')


def run(args, oracles, sh, Result):
    name = 'i386_symbol_resolution'
    ld = Path(args.lccc).resolve().with_name('lccc-ld')
    assembler = os.environ.get('LCCC_TEST_ASSEMBLER', 'as')
    bfd = shutil.which('ld.bfd') or shutil.which('ld')
    mold = shutil.which('mold') if any(n == 'mold' for n, _ in oracles) else None
    required = os.environ.get('LCCC_REQUIRE_I386') == '1'
    with tempfile.TemporaryDirectory(prefix='lnk.i386.symbols.') as directory:
        root = Path(directory)

        def invoke(cmd):
            return sh(cmd, cwd=directory)

        def link(linker, out, inputs, flags=()):
            return invoke([linker, '-m', 'elf_i386', *flags, '-o', out, *inputs])

        def diag(p):
            return p.stderr.decode(errors='replace').strip()

        src = {
            'probe': START + 'xor %ebx,%ebx\n' + EXIT,
            'call': START + 'call pick@PLT\nmov %eax,%ebx\n' + EXIT,
            'addr': START + 'mov $pick,%ebx\n' + EXIT,
            'weakaddr': '.weak pick\n' + START + 'mov $pick,%ebx\n' + EXIT,
            'got': START + ('call 1f\n1: pop %ebx\naddl $_GLOBAL_OFFSET_TABLE_+[.-1b],%ebx\n'
                            'mov pick@GOT(%ebx),%eax\nmov (%eax),%ebx\n') + EXIT,
            'a': function(3),
            'b': function(7) + '.globl pull_me\npull_me: ret\n',
            'w': function(5, '.weak'),
            'aa': '.globl pick\n.set pick,3\n',
            'ab': '.globl pick\n.set pick,7\n',
            'c4': '.comm pick,4,4\n',
            'cabs': '.comm pick,4,1\n',
            'c8': '.comm pick,8,128\n',
            'c64': '.comm pick,64,8\n',
            'd': data(7),
            'dw': data(5, '.weak'),
            'unique': data(9, kind='gnu_unique_object'),
            'ga': function(3, section='.section .text.pick,"axG",@progbits,same,comdat'),
            'gb': function(7, section='.section .text.pick,"axG",@progbits,same,comdat'),
            'gw': function(5, '.weak', '.section .text.pick,"axG",@progbits,same,comdat'),
            'gc': function(7, section='.section .text.pick,"axG",@progbits,other,comdat'),
            'lost': '.section .text.pick,"axG",@progbits,same,comdat\n.globl lost\nlost: ret\n',
            'lostref': ('.section .text.pick,"axG",@progbits,same,comdat\n.globl lost\nlost: ret\n'
                        '.data\n.long lost\n'),
            'lostlocal': ('.section .text.pick,"axG",@progbits,same,comdat\nlocal: ret\n'
                          '.data\n.long local\n'),
            'dead-undef': '.section .text.pick,"axG",@progbits,same,comdat\ncall absent@PLT\n',
            'live-undef': ('.section .text.pick,"axG",@progbits,same,comdat\ncall absent@PLT\n'
                           '.data\n.long absent\n'),
            'undef_weak': '.weak pick\n.data\n.long pick\n',
            'undef_strong': '.data\n.long pick\n',
        }
        # Probe the host assembler/loader separately. The fixture uses no
        # compiler multilib headers or runtime libraries.
        if not ld.is_file() or not shutil.which(assembler) or not bfd:
            return Result(name, 'FAIL' if required else 'SKIP', 'need lccc-ld, GNU as and ld')
        for stem, asm in src.items():
            (root / f'{stem}.s').write_text(asm + STACK)
            built = invoke([assembler, '--32', '-o', f'{stem}.o', f'{stem}.s'])
            if built.returncode != 0:
                status = 'FAIL' if required or stem != 'probe' else 'SKIP'
                return Result(name, status, f'{stem}.s: {diag(built)}')
        probe = link(bfd, 'probe', ['probe.o'])
        try:
            runnable = probe.returncode == 0 and invoke(['./probe']).returncode == 0
        except (OSError, RuntimeError) as e:
            return Result(name, 'FAIL' if required else 'SKIP', str(e))
        interp = '/lib/ld-linux.so.2'
        if not runnable or not os.path.isfile(interp):
            return Result(name, 'FAIL' if required else 'SKIP', 'cannot run ELF32 or missing ELF32 loader')
        ar = invoke(['ar', 'rcs', 'libpick.a', 'b.o'])
        if ar.returncode:
            return Result(name, 'FAIL', f'ar: {diag(ar)}')

        problems = []
        checked = 0

        def expect(label, inputs, flags=(), exit_code=None, error=None,
                   symbol=None, refs=('bfd', 'mold'), dso_caller='call.o'):
            nonlocal checked
            dso = '-shared' in flags
            pie = '-pie' in flags
            for who, linker in [('lccc', ld), ('bfd', bfd), ('mold', mold)]:
                if not linker or (who != 'lccc' and who not in refs):
                    continue
                checked += 1
                out = f'{who}-{label}' + ('.so' if dso else '')
                p = link(linker, out, inputs, flags)
                msg = diag(p)
                tag = f'{who}/{label}'
                if error:
                    needle = ('duplicate symbol' if who == 'mold' and error == 'multiple definition'
                              else error)
                    if not p.returncode or needle not in msg or (root / out).exists():
                        problems.append(f'{tag}: expected {needle!r} and no output, rc={p.returncode}: {msg}')
                    if (who == 'lccc' and error == 'multiple definition' and p.returncode
                            and not all(n in msg for n in ('pick', inputs[-2], inputs[-1]))):
                        problems.append(f'{tag}: diagnostic does not identify both inputs: {msg}')
                    continue
                if p.returncode or msg:
                    problems.append(f'{tag}: link rc={p.returncode}: {msg}')
                    continue
                header = (root / out).read_bytes()[:20]
                if header[:6] != b'\x7fELF\x01\x01' or header[16:20] != (b'\x03\0\x03\0'
                        if dso or pie else b'\x02\0\x03\0'):
                    problems.append(f'{tag}: wrong ELF32 machine or file type: {header.hex()}')
                if symbol:
                    sym, check = symbol
                    syms = invoke(['readelf', '--syms', '--wide', out])
                    rows = [line.split() for line in syms.stdout.decode().splitlines()
                            if line.split()[-1:] == [sym]]
                    if syms.returncode or not check(rows):
                        problems.append(f'{tag}: unexpected {sym} symbol rows {rows!r}')
                if exit_code is not None:
                    exe = out
                    if dso:
                        exe = out + '.exe'
                        p = link(linker, exe, [dso_caller, './' + out],
                                 ['--dynamic-linker', interp, '-rpath', '$ORIGIN'])
                        if p.returncode or diag(p):
                            problems.append(f'{tag}: DSO consumer: {diag(p)}')
                            continue
                    ran = invoke(['./' + exe])
                    if ran.returncode != exit_code or ran.stdout or ran.stderr:
                        problems.append(f'{tag}: expected exit {exit_code}, got {ran.returncode}: {diag(ran)}')

        pie = ('-pie', '--dynamic-linker', interp)
        for mode, flags in [('exec', ()), ('pie', pie), ('so', ('-shared',))]:
            entry = [] if mode == 'so' else ['call.o']
            for first, second, value in [('a.o', 'b.o', 3), ('b.o', 'a.o', 7)]:
                expect(f'{mode}-reject-{value}', entry + [first, second], flags,
                       error='multiple definition')
                for spelling, allow in [('long', ('--allow-multiple-definition',)),
                                        ('split', ('-z', 'muldefs')), ('joined', ('-zmuldefs',))]:
                    expect(f'{mode}-{spelling}-{value}', entry + [first, second],
                           (*flags, '--fatal-warnings', *allow), exit_code=value)
            for pos, inputs in [('first', ['w.o', 'b.o']), ('last', ['b.o', 'w.o'])]:
                expect(f'{mode}-weak-{pos}', entry + inputs, flags, exit_code=7)
            expect(f'{mode}-comdat-same', entry + ['ga.o', 'gb.o'], flags, exit_code=3)
            expect(f'{mode}-comdat-different', entry + ['ga.o', 'gc.o'], flags,
                   error='multiple definition')
        expect('archive-unselected', ['call.o', 'a.o', 'libpick.a'], exit_code=3)
        expect('archive-selected', ['call.o', 'a.o', 'libpick.a'], ['-u', 'pull_me'],
               error='multiple definition')
        expect('unique-strong', ['got.o', 'unique.o', 'd.o'], error='multiple definition')
        expect('absolute-identical', ['addr.o', 'aa.o', 'aa.o'], exit_code=3, refs=('bfd',))
        expect('absolute-different', ['addr.o', 'aa.o', 'ab.o'], error='multiple definition')
        expect('absolute-and-section', ['addr.o', 'aa.o', 'a.o'], error='multiple definition')
        expect('absolute-override', ['addr.o', 'ab.o', 'aa.o'], ['-z', 'muldefs'], exit_code=7)
        expect('absolute-vs-common', ['addr.o', 'cabs.o', 'aa.o'], exit_code=3)
        expect('common-vs-absolute', ['addr.o', 'aa.o', 'cabs.o'], exit_code=3)
        expect('discarded-strong-cannot-replace-live-weak', ['call.o', 'gw.o', 'gb.o'], exit_code=5)
        expect('discarded-symbol-not-absolute', ['call.o', 'ga.o', 'lost.o'], exit_code=3,
               symbol=('lost', lambda rows: not rows))
        expect('discarded-but-referenced', ['call.o', 'ga.o', 'lostref.o'], error='undefined', refs=('bfd',))
        expect('discarded-local-reference', ['probe.o', 'ga.o', 'lostlocal.o'],
               error='discarded', refs=('bfd',))
        expect('undef-only-in-discarded', ['probe.o', 'ga.o', 'dead-undef.o'], exit_code=0)
        expect('undef-also-in-kept', ['probe.o', 'ga.o', 'live-undef.o'], error='undefined')
        expect('dso-dead-undef-not-imported', ['ga.o', 'dead-undef.o'], ['-shared', '-z', 'defs'],
               symbol=('absent', lambda rows: not rows), refs=())
        expect('dso-weak-first-import-is-global', ['undef_weak.o', 'undef_strong.o'], ['-shared'],
               symbol=('pick', lambda rows: bool(rows) and all(r[4] == 'GLOBAL' and r[6] == 'UND'
                                                              for r in rows)), refs=('bfd',))
        expect('weak-only-undef', ['weakaddr.o', 'undef_weak.o'], exit_code=0)
        for mode, flags, entry in [('exec', (), ['probe.o']), ('pie', pie, ['probe.o']),
                                   ('so', ('-shared', '-z', 'defs'), [])]:
            for pos, inputs in [('first', ['undef_weak.o', 'undef_strong.o']),
                                ('last', ['undef_strong.o', 'undef_weak.o'])]:
                # GNU ld diagnoses both orders even for -shared -z defs.
                expect(f'{mode}-required-{pos}', [*entry, *inputs], flags, error='undefined',
                       refs=('bfd',) if mode == 'so' else ('bfd', 'mold'))
            for pos, inputs in [('first', ['c4.o', 'd.o']), ('last', ['d.o', 'c4.o'])]:
                expect(f'{mode}-common-strong-{pos}', inputs if mode == 'so' else ['got.o', *inputs],
                       flags, exit_code=7, dso_caller='got.o')
        for pos, inputs in [('first', ['c8.o', 'c64.o']), ('last', ['c64.o', 'c8.o'])]:
            expect(f'common-max-{pos}', ['got.o', *inputs], exit_code=0,
                   symbol=('pick', lambda rows: len(rows) == 1 and int(rows[0][2]) == 64
                           and int(rows[0][1], 16) % 128 == 0 and rows[0][6].isdigit()))
        for pos, inputs in [('first', ['c4.o', 'dw.o']), ('last', ['dw.o', 'c4.o'])]:
            # GNU ld, unlike mold, places a tentative strong definition
            # above a weak initialized definition. Match our existing ABI.
            expect(f'common-weak-{pos}', ['got.o', *inputs], exit_code=0, refs=('bfd',))
        return Result(name, 'FAIL' if problems else 'PASS', '\n'.join(problems) if problems else
                      f'{checked} same-object checks (static executable, PIE, DSO; bfd'
                      + (', mold)' if mold else ')'))
