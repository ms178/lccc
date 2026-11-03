#!/usr/bin/env python3
"""GCC oracle for reproducibility diagnostics and builtin-origin restoration."""
from __future__ import annotations
import argparse
import os
import re
import shlex
import subprocess
import tempfile
from pathlib import Path

FIXTURES = [
    ('date', 'const char *x = __DATE__;\n', 1),
    ('time', 'const char *x = __TIME__;\n', 1),
    ('timestamp', 'const char *x = __TIMESTAMP__;\n', 1),
    ('alias', '#define VALUE __DATE__\nconst char *x = VALUE;\n', 1),
    ('unused', '#define UNUSED(x) 0\nint x = UNUSED(__DATE__);\n', 0),
    ('stringified', '#define STRING(x) #x\nconst char *x = STRING(__DATE__);\n', 0),
    ('disabled', '#if 0\nconst char *x = __TIME__;\n#endif\nint y;\n', 0),
    ('redefined', '#undef __DATE__\n#define __DATE__ "stable"\nconst char *x = __DATE__;\n', 0),
    ('restored', '#pragma push_macro("__DATE__")\n#undef __DATE__\n#define __DATE__ "stable"\nconst char *x = __DATE__;\n#pragma pop_macro("__DATE__")\nconst char *y = __DATE__;\n', 1),
    ('unused-pragma', '#define UNUSED(x) 0\nint y = UNUSED(_Pragma("GCC diagnostic ignored \\\"-Wdate-time\\\""));\nconst char *x = __DATE__;\n', 1),
    ('paste', '#define JOIN(a,b) a##b\nconst char *x = JOIN(__DA,TE__);\n', 1),
    ('va-presence', '#define F(...) __VA_OPT__(0)\nint x=F(__DATE__);\n', 1),
    ('va-empty', '#define E\n#define F(...) 0 __VA_OPT__(+1)\nint x=F(E);\n', 0),
    ('va-omitted', '#define F(...) 0 __VA_OPT__(+sizeof(__DATE__))\nint x=F();\n', 0),
    ('two', 'const char *x = __DATE__, *y = __TIME__;\n', 2),
]


# All three dynamic builtins must use the ownership-aware warning path, not a
# flag on their (now deliberately empty) macro replacement definitions.
for builtin in ('__TIME__', '__TIMESTAMP__'):
    for name, text, count in list(FIXTURES):
        if name in ('alias', 'unused', 'stringified', 'redefined', 'restored'):
            FIXTURES.append((f'{name}-{builtin}', text.replace('__DATE__', builtin), count))


def run(lccc: list[str], gcc: list[str], clang: list[str] | None = None) -> None:
    # check_warning_precedence imports the historical two-argument API.
    # Keep that caller compatible without dropping the mandatory Clang controls.
    if clang is None:
        clang = shlex.split(os.environ.get('CLANG', 'clang-23'))
    env = {**os.environ, 'SOURCE_DATE_EPOCH': '0', 'TZ': 'UTC', 'LC_ALL': 'C'}
    flags = [([], None), (['-Wall', '-Wextra'], None), (['-Wdate-time'], 'warning'),
             (['-Werror=date-time'], 'error'), (['-Werror=date-time', '-Wno-error=date-time'], 'warning'),
             (['-Wdate-time', '-Wno-date-time'], None), (['-w', '-Werror=date-time'], None)]
    checks = 0
    with tempfile.TemporaryDirectory(prefix='lccc-date-time-') as td:
        root = Path(td)
        source = root / 'fixture.c'
        for name, text, count in FIXTURES:
            source.write_text(text)
            for options, severity in flags:
                for mode in [['-E'], ['-c'], ['-fsyntax-only'], ['-M'], ['-MM'], ['-E', '-dM']]:
                    expected = [severity] * count if severity and mode not in (['-M'], ['-MM'], ['-E', '-dM']) else []
                    for label, compiler in [('GCC', gcc), ('LCCC', lccc)]:
                        proc = subprocess.run(compiler + options + mode + [str(source), '-o', str(root / 'out')],
                            text=True, capture_output=True, env=env, timeout=30)
                        actual = re.findall(r'(warning|error):[^\n]*\[-W(?:error=)?date-time\]', proc.stderr)
                        if actual != expected or bool(proc.returncode) != ('error' in expected):
                            raise RuntimeError(f'{label} {name} {options} {mode}: expected {expected}, got {actual}, exit {proc.returncode}\n{proc.stderr}')
                    checks += 1
        source.write_text('const char *a = __DATE__; _Pragma("GCC diagnostic ignored \\"-Wdate-time\\"") const char *b = __TIME__;\n')
        for label, compiler in [('GCC', gcc), ('LCCC', lccc)]:
            proc = subprocess.run(compiler + ['-E', '-Wdate-time', str(source)],
                text=True, capture_output=True, env=env, timeout=30)
            if proc.returncode or proc.stderr.count('[-Wdate-time]') != 1:
                raise RuntimeError(f'{label} same-line source ordering: {proc.stderr}')
        checks += 1
        # With no reproducibility epoch, physical mtime ignores a #line name.
        source.write_text('#line 800 "invented.c"\nconst char *x = __TIMESTAMP__;\n')
        os.utime(source, (86400, 86400))
        physical_env = {key: value for key, value in env.items() if key != 'SOURCE_DATE_EPOCH'}
        for label, compiler in [('GCC', gcc), ('LCCC', lccc), ('Clang', clang)]:
            proc = subprocess.run(compiler + ['-E', '-P', str(source)],
                text=True, capture_output=True, env=physical_env, timeout=30)
            if proc.returncode or '"Fri Jan  2 00:00:00 1970"' not in proc.stdout:
                raise RuntimeError(f'{label} physical timestamp: {proc.stdout}\n{proc.stderr}')
        checks += 1
        # Upstream adopts Clang's reproducible timestamp rule; GCC deliberately
        # ignores the epoch for this macro and remains the no-epoch oracle above.
        for label, compiler in [('LCCC', lccc), ('Clang', clang)]:
            proc = subprocess.run(compiler + ['-E', '-P', str(source)],
                text=True, capture_output=True, env=env, timeout=30)
            if proc.returncode or '"Thu Jan  1 00:00:00 1970"' not in proc.stdout:
                raise RuntimeError(f'{label} reproducible timestamp: {proc.stdout}\n{proc.stderr}')
        checks += 1
        # Every builtin, across compiler-owned, undefined and user-owned states.
        names = ['__FILE__', '__BASE_FILE__', '__DATE__', '__TIME__', '__LINE__',
                 '__COUNTER__', '__INCLUDE_LEVEL__', '__TIMESTAMP__']
        for name in names:
            for state in ['compiler', 'undefined', 'user']:
                prefix = '' if state == 'compiler' else f'#undef {name}\n'
                if state == 'user':
                    prefix += f'#define {name} "original user body"\n'
                source.write_text(prefix + f'#pragma push_macro("{name}")\n'
                    f'#undef {name}\n#define {name} "temporary body"\n'
                    f'#pragma pop_macro("{name}")\n#ifdef {name}\nint available=1;\n#endif\n')
                for label, compiler in [('GCC', gcc), ('LCCC', lccc)]:
                    for mode in [['-P'], ['-dM']]:
                        proc = subprocess.run(compiler + ['-E', *mode, str(source)],
                            text=True, capture_output=True, env=env, timeout=30)
                        present = (bool(re.search(r'^#define ' + name + r'\b', proc.stdout, re.M))
                                   if mode == ['-dM'] else 'int available=1;' in proc.stdout.replace(' = ', '='))
                        expected = state == 'user' if mode == ['-dM'] else state != 'undefined'
                        wrong_body = state == 'user' and mode == ['-dM'] and not re.search(
                            r'^#define ' + name + r'\s+"original user body"$', proc.stdout, re.M)
                        if proc.returncode or present != expected or wrong_body:
                            raise RuntimeError(f'{label} restore {name}/{state}/{mode}: {proc.stdout}\n{proc.stderr}')
                checks += 2
        source.write_text('__COUNTER__\n#pragma push_macro("__COUNTER__")\n'
            '#undef __COUNTER__\n#define __COUNTER__ 88\n#pragma push_macro("__COUNTER__")\n'
            '#undef __COUNTER__\n#pragma pop_macro("__COUNTER__")\n__COUNTER__\n'
            '#pragma pop_macro("__COUNTER__")\n__COUNTER__\n')
        for label, compiler in [('GCC', gcc), ('LCCC', lccc)]:
            proc = subprocess.run(compiler + ['-E', '-P', str(source)],
                text=True, capture_output=True, env=env, timeout=30)
            if proc.returncode or proc.stdout.split() != ['0', '88', '1']:
                raise RuntimeError(f'{label} nested counter restoration: {proc.stdout}\n{proc.stderr}')
        checks += 1
        source.write_text('#undef __FILE__\n#define __FILE__ "not-the-diagnostic-file"\n'
            'const char *x = __DATE__;\n')
        for label, compiler in [('GCC', gcc), ('LCCC', lccc)]:
            proc = subprocess.run(compiler + ['-E', '-Wdate-time', str(source)],
                text=True, capture_output=True, env=env, timeout=30)
            if proc.returncode or f'{source}:3:' not in proc.stderr:
                raise RuntimeError(f'{label} independent diagnostic provenance: {proc.stderr}')
        checks += 1
        # Restoring the compiler FILE inside a header must use the header's path.
        header = root / 'restore.h'
        header.write_text('#pragma pop_macro("__FILE__")\nconst char *inside = __FILE__;\n')
        source.write_text('#pragma push_macro("__FILE__")\n#undef __FILE__\n'
            '#define __FILE__ "temporary"\n#include "restore.h"\n')
        for label, compiler in [('GCC', gcc), ('LCCC', lccc)]:
            proc = subprocess.run(compiler + ['-E', '-P', str(source)],
                text=True, capture_output=True, env=env, timeout=30)
            if proc.returncode or f'"{header}"' not in proc.stdout:
                raise RuntimeError(f'{label} restored FILE use site: {proc.stdout}\n{proc.stderr}')
        checks += 1
        for text, listed in [('', False), ('#undef __DATE__\n#define __DATE__ "stable"\n', True),
            ('#pragma push_macro("__DATE__")\n#undef __DATE__\n#define __DATE__ "stable"\n#pragma pop_macro("__DATE__")\n', False)]:
            source.write_text(text)
            for label, compiler in [('GCC', gcc), ('LCCC', lccc)]:
                proc = subprocess.run(compiler + ['-E', '-dM', str(source)], text=True, capture_output=True, env=env, timeout=30)
                present = bool(re.search(r'^#define __DATE__\b', proc.stdout, re.M))
                if proc.returncode or present != listed or re.search(r'^#define __(TIME|TIMESTAMP)__\b', proc.stdout, re.M):
                    raise RuntimeError(f'{label} builtin-origin macro dump: {proc.stdout}\n{proc.stderr}')
            checks += 1
        source.write_text('#define IGNORE(x) 0\n#define STR(x) #x\n#define RAW(x) x##_tag\n#define TWICE(x) x,x\n#define REV(a,b) b,a\nIGNORE(__COUNTER__) __COUNTER__\nSTR(__COUNTER__) __COUNTER__\nRAW(__COUNTER__) __COUNTER__\nTWICE(__COUNTER__) __COUNTER__\nREV(__COUNTER__,__COUNTER__) __COUNTER__\n#define VP(a,...) a __VA_OPT__(__VA_ARGS__)\nVP(__COUNTER__,__COUNTER__) __COUNTER__\n#define VR(a,...) __VA_OPT__(__VA_ARGS__) a\nVR(__COUNTER__,__COUNTER__) __COUNTER__\n#define E\n#define NONEMPTY(...) __VA_OPT__(1)\nNONEMPTY(,) NONEMPTY(E)\n')
        expected = ['0', '0', '"__COUNTER__"', '1', '__COUNTER___tag', '2', '3', ',', '3', '4', '5', ',', '6', '7', '8', '9', '10', '11', '12', '13', '1']
        for label, compiler in [('GCC', gcc), ('LCCC', lccc)]:
            proc = subprocess.run(compiler + ['-E', '-P', str(source)], text=True, capture_output=True, env=env, timeout=30)
            tokens = re.findall(r'"[^"\n]*"|[A-Za-z_]\w*|\d+|[^\s]', proc.stdout)
            if proc.returncode or tokens != expected:
                raise RuntimeError(f'{label} argument prescan side effects: {tokens}\n{proc.stderr}')
        checks += 1
    print(f'date/time diagnostics: PASS ({checks} checks against non-kernel GCC; Clang timestamp controls)')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lccc', default='target/fastbuild/lccc')
    parser.add_argument('--gcc', default='gcc-16')
    parser.add_argument('--clang', default='clang-23')
    args = parser.parse_args()
    run(shlex.split(args.lccc), shlex.split(args.gcc), shlex.split(args.clang))
