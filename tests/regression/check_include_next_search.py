#!/usr/bin/env python3
"""Differential include-next provenance and search-order contracts.

All fixtures are self-contained; -nostdinc makes their search lists independent
of the host. GCC is the primary oracle for quoted-current-directory behavior,
where GCC and Clang intentionally differ. No generated file escapes the temp
root and a failed compiler or missing header is never treated as a match.
"""
import argparse
import re
import shutil
import subprocess
import tempfile
from pathlib import Path


def markers(output):
    return re.findall(r'\bMARK_[A-Z0-9_]+\b', output)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--lccc', default='target/fastbuild/lccc')
    parser.add_argument('--gcc', default='gcc-16')
    parser.add_argument('--clang', default='')
    args = parser.parse_args()
    compilers = [('lccc', str(Path(args.lccc).resolve())), ('gcc', args.gcc)]
    if args.clang:
        if not shutil.which(args.clang):
            parser.error(f'clang not found: {args.clang}')
        compilers.append(('clang', args.clang))

    with tempfile.TemporaryDirectory(prefix='lccc-include-next-') as tmp:
        root = Path(tmp)
        a, b, c, src = (root / name for name in ('a', 'b', 'c', 'src'))
        for directory in (a, b, c, src, a / 'sub'):
            directory.mkdir(parents=True, exist_ok=True)
        link = root / 'alias-a'
        link.symlink_to(a, target_is_directory=True)
        main_file = src / 'unit.c'

        def write(path, text):
            path.write_text(text + '\n', encoding='utf-8')

        def check(name, source, flags, expected, clang=True, fails=False):
            write(main_file, source)
            for mode in ('-m64', '-m32'):
                for label, compiler in compilers:
                    if label == 'clang' and not clang:
                        continue
                    cmd = [compiler, mode, '-E', '-P', '-nostdinc', *flags, str(main_file)]
                    result = subprocess.run(cmd, text=True, capture_output=True, timeout=20, cwd=root)
                    if fails:
                        if result.returncode == 0:
                            raise AssertionError(f'{name}/{mode}/{label}: missing header was accepted')
                    elif result.returncode != 0 or markers(result.stdout) != expected:
                        raise AssertionError(
                            f'{name}/{mode}/{label}: rc={result.returncode}, '
                            f'markers={markers(result.stdout)!r}, expected={expected!r}\n{result.stderr[-1500:]}'
                        )
            print('PASS', name)

        write(a / 'first.h', 'MARK_A\n#include_next <other.h>')
        write(a / 'other.h', 'MARK_A_OTHER')
        write(b / 'other.h', 'MARK_B_OTHER')
        write(c / 'other.h', 'MARK_C_OTHER')
        check('different header name', '#include <first.h>', ['-I' + str(a), '-I' + str(b)],
              ['MARK_A', 'MARK_B_OTHER'])
        check('quote directories after origin', '#include "first.h"',
              ['-iquote', str(a), '-iquote', str(c), '-I' + str(b)],
              ['MARK_A', 'MARK_C_OTHER'])

        write(a / 'same.h', 'MARK_A\n#include_next <same.h>')
        write(b / 'same.h', 'MARK_B\n#if __has_include_next(<same.h>)\n#include_next <same.h>\n#else\nMARK_END\n#endif')
        check('duplicate directories', '#include <same.h>',
              ['-I' + str(a), '-I' + str(b), '-I' + str(a), '-I' + str(link)],
              ['MARK_A', 'MARK_B', 'MARK_END'])
        # The same directory can legitimately occur in both quote and -I
        # chains. GCC removes the boundary duplicate before deduplicating
        # repeated -iquote entries, not afterwards.
        write(a / 'same.h', 'MARK_A\n#if __has_include_next(<same.h>)\n#include_next <same.h>\n#else\nMARK_END\n#endif')
        check('repeated quote at bracket join', '#include "same.h"',
              ['-iquote', str(a), '-iquote', str(a), '-I' + str(a), '-I' + str(b)],
              ['MARK_A', 'MARK_A', 'MARK_B', 'MARK_END'], clang=False)
        check('quote directory superseded by system', '#include "same.h"',
              ['-iquote', str(a), '-iquote', str(b), '-isystem', str(a), '-I' + str(b)],
              ['MARK_B', 'MARK_A', 'MARK_END'], clang=False)

        write(a / 'same.h', 'MARK_A')
        write(b / 'same.h', 'MARK_B')
        check('system directory supersedes -I', '#include <same.h>',
              ['-I' + str(a), '-I' + str(b), '-isystem', str(a)], ['MARK_B'])
        check('after directory supersedes -I', '#include <same.h>',
              ['-I' + str(a), '-I' + str(b), '-idirafter', str(a)], ['MARK_B'])

        write(a / 'sub' / 'first.h', 'MARK_A\n#include_next <other.h>')
        check('subdirectory origin', '#include <sub/first.h>', ['-I' + str(a), '-I' + str(b)],
              ['MARK_A', 'MARK_B_OTHER'])
        write(a / 'first.h', 'MARK_FIRST\n#include "sub/child.h"')
        write(a / 'sub' / 'child.h', 'MARK_CHILD\n#include_next <other.h>')
        check('quoted child starts a new local search', '#include <first.h>',
              ['-I' + str(a), '-I' + str(b)], ['MARK_FIRST', 'MARK_CHILD', 'MARK_A_OTHER'],
              clang=False)
        write(src / 'local.h', 'MARK_LOCAL\n#include_next <other.h>')
        check('local header searches quote chain', '#include "local.h"',
              ['-iquote', str(c), '-I' + str(b)], ['MARK_LOCAL', 'MARK_C_OTHER'], clang=False)

        write(a / 'force.h', 'MARK_A\n#include_next <other.h>')
        # GCC's -include starts at CWD, not the primary source directory.
        write(src / 'force.h', 'MARK_WRONG_SOURCE_DIRECTORY')
        check('force-include provenance', 'MARK_MAIN',
              ['-I' + str(a), '-I' + str(b), '-include', 'force.h'],
              ['MARK_A', 'MARK_B_OTHER', 'MARK_MAIN'])
        write(a / 'absolute.h', 'MARK_A\n#include_next "' + str(b / 'other.h') + '"')
        check('absolute include-next', '#include <absolute.h>', ['-I' + str(a)],
              ['MARK_A', 'MARK_B_OTHER'], clang=False)
        write(a / 'exhausted.h', '#include_next <exhausted.h>')
        check('exhaustion diagnoses missing header', '#include <exhausted.h>',
              ['-I' + str(a), '-I' + str(link)], [], fails=True)

        # Two GCC-style wrappers defer their common guard until AFTER the
        # next header. Revisiting an earlier directory creates a silent loop
        # and loses the final header's definitions.
        wrapper = '#ifndef WRAP_DONE\n#include_next <types.h>\n#define WRAP_DONE\n#endif'
        write(a / 'types.h', 'MARK_WRAP_A\n' + wrapper)
        write(b / 'types.h', 'MARK_WRAP_B\n' + wrapper)
        write(c / 'types.h', 'MARK_TYPES\n#define TYPE_VALUE 7')
        check('nested wrapper chain', '#include <types.h>\nMARK_DONE',
              ['-I' + str(a), '-isystem', str(b), '-idirafter', str(c), '-I' + str(a)],
              ['MARK_WRAP_A', 'MARK_WRAP_B', 'MARK_TYPES', 'MARK_DONE'])

    print('OK: include-next provenance, duplicate paths, and exhaustion')


if __name__ == '__main__':
    main()
