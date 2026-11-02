#!/usr/bin/env python3
"""GNU-oracle orphan-policy tests for the ELF32/64 explicit-script linker.

Only assembler fixtures, never GCC-built kernel objects. Non-script non-default
policies must fail explicitly until that independent backend supports them.
"""
from pathlib import Path
import os
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
META = '*(.got .got.plt .igot.plt .iplt .rel.* .rela.* .note.gnu.property)'
SCRIPT = ('ENTRY(_start) SECTIONS { . = 0x10000; .text : { *(.text*) } '
          '.data : { *(.data*) } .bss : { *(.bss*) } .meta : { ' + META + ' } '
          '/DISCARD/ : { *(.note.GNU-stack) DISCARDS } EXTRA_RULE }')


def command(argv):
    return subprocess.run(list(map(str, argv)), capture_output=True, text=True,
                          timeout=60, env=dict(os.environ, LC_ALL='C'))


def run(ld=None):
    ld = Path(ld or ROOT / 'target/fastbuild/lccc-ld').resolve()
    assembler = shutil.which('as')
    oracle = shutil.which('ld.bfd')
    readelf = shutil.which('readelf')
    assert assembler and oracle and readelf, 'as, GNU ld.bfd and readelf required'
    measured = 0
    with tempfile.TemporaryDirectory(prefix='lccc-orphan-policy-') as directory:
        work = Path(directory)
        for bits in (32, 64):
            emulation = 'elf_i386' if bits == 32 else 'elf_x86_64'
            source, obj, script = work/'a.s', work/'a.o', work/'a.ld'
            for name, content in (
                ('matched', ''),
                ('alloc', '.section .extra,"a"\n.byte 17\n'),
                ('nonalloc', '.section .debug_extra,""\n.byte 23\n'),
                ('empty', '.section .empty_extra,"a"\n'),
                ('excluded', '.section .excluded,"e"\n.byte 29\n'),
            ):
                source.write_text('.text\n.globl _start\n_start: ret\n' + content +
                                  '.section .note.GNU-stack,"",@progbits\n')
                built = command([assembler, f'--{bits}', '-o', obj, source])
                assert built.returncode == 0, built.stderr
                script.write_text(SCRIPT.replace('DISCARDS', '').replace('EXTRA_RULE', ''))
                for mode in ('place', 'warn', 'error', 'discard'):
                    for fatal in (False, True):
                        opts = [f'--orphan-handling={mode}']
                        if fatal:
                            opts += ['--fatal-warnings']
                        results = []
                        for linker in (str(ld), oracle):
                            output = work / ('lccc' if linker == str(ld) else 'gnu')
                            output.unlink(missing_ok=True)
                            result = command([linker, '-m', emulation, *opts,
                                              '-T', script, '-o', output, obj])
                            results.append(result)
                            if mode == 'discard' and result.returncode == 0:
                                sections = command([readelf, '-SW', output])
                                assert sections.returncode == 0, sections.stderr
                                for orphan in ('.extra', '.debug_extra', '.empty_extra'):
                                    assert orphan not in sections.stdout, sections.stdout
                        assert bool(results[0].returncode) == bool(results[1].returncode), (
                            bits, name, opts, [(r.returncode, r.stderr) for r in results])
                        if name in ('alloc', 'nonalloc', 'empty') and mode in ('warn', 'error'):
                            assert 'orphan section' in results[0].stderr, results[0].stderr
                        elif results[0].returncode == 0:
                            assert 'orphan section' not in results[0].stderr, results[0].stderr
                        measured += 1
                # Explicit discard/wildcard rules are NOT orphans, including
                # sections with no payload. Error mode must not reject these.
                for rule in ('/DISCARD/ : { *(.extra .debug_extra .empty_extra) }',
                             '.selected : { *(.extra .debug_extra .empty_extra) }'):
                    script.write_text(SCRIPT.replace('DISCARDS', '').replace('EXTRA_RULE', rule))
                    for linker in (str(ld), oracle):
                        result = command([linker, '-m', emulation, '--orphan-handling=error',
                                          '-T', script, '-o', work/'selected', obj])
                        assert result.returncode == 0, (bits, name, rule, result.stderr)
                    measured += 1
            # References and --defsym aliases into discarded sections must
            # fail, never quietly bind to zero or a stale pre-discard address.
            source.write_text('.text\n.globl _start\n_start: .long extra_symbol\n'
                              '.section .extra,"a"\n.globl extra_symbol\n'
                              'extra_symbol: .long 9\n'
                              '.section .note.GNU-stack,"",@progbits\n')
            assert command([assembler, f'--{bits}', '-o', obj, source]).returncode == 0
            script.write_text(SCRIPT.replace('DISCARDS', '').replace('EXTRA_RULE', ''))
            for definitions in ([], ['--defsym=copy=extra_symbol']):
                for linker in (str(ld), oracle):
                    result = command([linker, '-m', emulation, '--orphan-handling=discard',
                                      *definitions, '-T', script, '-o', work/'dangling', obj])
                    assert result.returncode != 0, (bits, definitions, linker, result.stderr)
                measured += 1
            # GNU intentionally keeps a --defsym alias of discarded input
            # as an absolute offset. Also exercise a deferred expression
            # mixing that offset with a live, not-yet-placed address.
            source.write_text('.text\n.globl _start\n_start: .long copy\nret\n'
                              '.section .extra,"a"\n.long 0\n.globl extra_symbol\n'
                              'extra_symbol: .long 9\n'
                              '.section .note.GNU-stack,"",@progbits\n')
            assert command([assembler, f'--{bits}', '-o', obj, source]).returncode == 0
            for expression, expected in (('extra_symbol', 4), ('extra_symbol+7', 11),
                                         ('extra_symbol+_start+1', 0x10005)):
                for linker in (str(ld), oracle):
                    output = work/'alias'
                    result = command([linker, '-m', emulation, '--orphan-handling=discard',
                                      f'--defsym=copy={expression}', '-T', script,
                                      '-o', output, obj])
                    assert result.returncode == 0, (expression, linker, result.stderr)
                    symbols = command([readelf, '-sW', output])
                    rows = [line.split() for line in symbols.stdout.splitlines()
                            if line.split() and line.split()[-1] == 'copy']
                    assert len(rows) == 1 and int(rows[0][1], 16) == expected, symbols.stdout
                measured += 1
            # A discarded strong winner does not resurrect a weak loser.
            source.write_text('.text\n.globl _start\n_start: .long extra_symbol\n'
                              '.section .extra,"a"\n.globl extra_symbol\nextra_symbol: .long 9\n'
                              '.section .note.GNU-stack,"",@progbits\n')
            assert command([assembler, f'--{bits}', '-o', obj, source]).returncode == 0
            weak_source, weak = work/'weak.s', work/'weak.o'
            weak_source.write_text('.text\n.weak extra_symbol\nextra_symbol: ret\n'
                                   '.section .note.GNU-stack,"",@progbits\n')
            assert command([assembler, f'--{bits}', '-o', weak, weak_source]).returncode == 0
            for linker in (str(ld), oracle):
                result = command([linker, '-m', emulation, '--orphan-handling=discard',
                                  '-T', script, '-o', work/'weak', obj, weak])
                assert result.returncode != 0, (linker, result.stderr)
            measured += 1
            # FDEs describing discarded code must be pruned before layout.
            source.write_text('.text\n.globl _start\n_start:\n.cfi_startproc\n'
                              'ret\n.cfi_endproc\n.section .extra,"ax"\n'
                              'extra_function:\n.cfi_startproc\nret\n.cfi_endproc\n'
                              '.section .note.GNU-stack,"",@progbits\n')
            assert command([assembler, f'--{bits}', '-o', obj, source]).returncode == 0
            script.write_text(SCRIPT.replace('DISCARDS', '').replace(
                'EXTRA_RULE', '.eh_frame : { *(.eh_frame) }'))
            for linker in (str(ld), oracle):
                output = work/'frames'
                result = command([linker, '-m', emulation, '--orphan-handling=discard',
                                  '-T', script, '-o', output, obj])
                assert result.returncode == 0, result.stderr
                frames = command([readelf, '--debug-dump=frames', output])
                assert frames.returncode == 0 and frames.stdout.count(' FDE cie=') == 1, frames
            measured += 1
            # Last policy wins; malformed values never become filenames.
            source.write_text('.text\n.globl _start\n_start: ret\n'
                              '.section .extra,"a"\n.byte 7\n'
                              '.section .note.GNU-stack,"",@progbits\n')
            assert command([assembler, f'--{bits}', '-o', obj, source]).returncode == 0
            script.write_text(SCRIPT.replace('DISCARDS', '').replace('EXTRA_RULE', ''))
            for modes, expected in ((['error', 'place'], 0), (['place', 'error'], 1)):
                result = command([ld, '-m', emulation, *[f'--orphan-handling={m}' for m in modes],
                                  '-T', script, '-o', work/'last', obj])
                assert bool(result.returncode) == bool(expected), result.stderr
                measured += 1
            for value in ('', 'bogus', 'ERROR'):
                result = command([ld, f'--orphan-handling={value}', '-T', script, obj])
                assert result.returncode and 'invalid argument' in result.stderr, result.stderr
                measured += 1
            separated = command([ld, '-m', emulation, '--orphan-handling', 'error',
                                 '-T', script, '-o', work/'separated', obj])
            assert separated.returncode and 'unplaced orphan section' in separated.stderr
            # Garbage-collected allocatable orphans do not cause error mode
            # to fail. The oracle enforces the order of GC versus diagnostics.
            for linker in (str(ld), oracle):
                result = command([linker, '-m', emulation, '--gc-sections',
                                  '--orphan-handling=error', '-T', script,
                                  '-o', work/'gc', obj])
                assert result.returncode == 0, result.stderr
            measured += 2
        missing = command([ld, '--orphan-handling'])
        assert missing.returncode and 'missing argument' in missing.stderr
        non_script = command([ld, '--orphan-handling=error', obj])
        assert non_script.returncode and 'requires an explicit linker script' in non_script.stderr
    print(f'orphan policy: PASS ({measured} ELF32/64 mode/fatal/match/GC checks with GNU ld)')


if __name__ == '__main__':
    run()
