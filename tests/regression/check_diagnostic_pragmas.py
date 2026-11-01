#!/usr/bin/env python3
"""Non-kernel GCC 16.2 oracle: scoped diagnostic state and exact source lines."""
from __future__ import annotations
import argparse
import re
import shlex
import subprocess
import tempfile
from pathlib import Path

# Inline expectation comments freeze severity AND location, rather than merely
# accepting any warning somewhere in a translation unit. No kernel is built by GCC.
CASES = [
    ('ignored', 'missing-prototypes', ['-Werror=missing-prototypes'],
     '#pragma GCC diagnostic ignored "-Wmissing-prototypes"\nint f(void) { return 0; }\n'),
    ('push-pop', 'missing-prototypes', ['-Wmissing-prototypes'],
     '#pragma GCC diagnostic push\n#pragma GCC diagnostic ignored "-Wmissing-prototypes"\nint f(void) { return 0; }\n#pragma GCC diagnostic pop\nint g(void) { return 0; } /*expect-W*/\n'),
    ('nested', 'missing-prototypes', ['-Wmissing-prototypes'],
     '#pragma GCC diagnostic push\n#pragma GCC diagnostic ignored "-Wmissing-prototypes"\n#pragma GCC diagnostic push\n#pragma GCC diagnostic error "-Wmissing-prototypes"\nint f(void) { return 0; } /*expect-E*/\n#pragma GCC diagnostic pop\nint g(void) { return 0; }\n#pragma GCC diagnostic pop\nint h(void) { return 0; } /*expect-W*/\n'),
    ('warning-demotes', 'missing-prototypes', ['-Werror=missing-prototypes'],
     '#pragma GCC diagnostic warning "-Wmissing-prototypes"\nint f(void) { return 0; } /*expect-W*/\n'),
    ('error-enables', 'missing-prototypes', ['-Wno-missing-prototypes'],
     '#pragma GCC diagnostic error "-Wmissing-prototypes"\nint f(void) { return 0; } /*expect-E*/\n'),
    ('warning-enables', 'missing-prototypes', ['-Wno-missing-prototypes'],
     '#pragma GCC diagnostic warning "-Wmissing-prototypes"\nint f(void) { return 0; } /*expect-W*/\n'),
    ('unmatched-pop-baseline', 'missing-prototypes', ['-Werror=missing-prototypes'],
     '#pragma GCC diagnostic ignored "-Wmissing-prototypes"\n#pragma GCC diagnostic pop\nint f(void) { return 0; } /*expect-E*/\n'),
    ('mute-global', 'missing-prototypes', ['-w', '-Werror=missing-prototypes'],
     '#pragma GCC diagnostic error "-Wmissing-prototypes"\nint f(void) { return 0; }\n'),
    ('same-line', 'missing-prototypes', ['-Wmissing-prototypes'],
     '_Pragma("GCC diagnostic push") _Pragma("GCC diagnostic ignored \\"-Wmissing-prototypes\\"") int f(void) { return 0; } _Pragma("GCC diagnostic pop") int g(void) { return 0; } /*expect-W*/\n'),
    ('macro-pragmas', 'missing-prototypes', ['-Wmissing-prototypes'],
     '#define P(x) _Pragma(#x)\n#define BEGIN P(GCC diagnostic push) P(GCC diagnostic ignored "-Wmissing-prototypes")\n#define END P(GCC diagnostic pop)\nBEGIN int f(void) { return 0; } END\nint g(void) { return 0; } /*expect-W*/\n'),
    ('keyword-macros', 'missing-prototypes', ['-Wmissing-prototypes'],
     '#define GCC wrong\n#define diagnostic wrong\n#define ignored wrong\n_Pragma("GCC diagnostic ignored \\"-Wmissing-prototypes\\"") int f(void) { return 0; }\n'),
    ('ignored-prototypes-no-fallback', 'missing-declarations', ['-Wmissing-prototypes', '-Wmissing-declarations'],
     '#pragma GCC diagnostic ignored "-Wmissing-prototypes"\nint f(void) { return 0; }\n'),
    ('prototypes-analysis-position', 'missing-declarations', ['-Wmissing-declarations'],
     'int f(void) { return 0; } /*expect-W*/\n#pragma GCC diagnostic push\n#pragma GCC diagnostic warning "-Wmissing-prototypes"\nint g(void) { return 0; }\n#pragma GCC diagnostic pop\nint h(void) { return 0; }\n'),
    ('lexer-scopes', 'multichar', ['-Wmultichar'],
     '#pragma GCC diagnostic push\n#pragma GCC diagnostic ignored "-Wmultichar"\nint x = \'ab\';\n#pragma GCC diagnostic pop\nint y = \'cd\'; /*expect-W*/\n'),
    ('parser-scopes', 'strict-prototypes', ['-Wstrict-prototypes'],
     '#pragma GCC diagnostic push\n#pragma GCC diagnostic ignored "-Wstrict-prototypes"\nint f();\n#pragma GCC diagnostic pop\nint g(); /*expect-W*/\n'),
    ('pp-scopes', 'undef', ['-Wundef'],
     '#pragma GCC diagnostic push\n#pragma GCC diagnostic ignored "-Wundef"\n#if FIRST\n#endif\n#pragma GCC diagnostic pop\n#if SECOND /*expect-W*/\n#endif\n'),
    ('pp-operator', 'undef', ['-Wundef'],
     '_Pragma("GCC diagnostic ignored \\"-Wundef\\"")\n#if FIRST\n#endif\n'),
    ('pp-demote', 'undef', ['-Werror=undef'],
     '#pragma GCC diagnostic warning "-Wundef"\n#if FIRST /*expect-W*/\n#endif\n'),
    ('pp-promote', 'undef', ['-Wno-undef'],
     '#pragma GCC diagnostic error "-Wundef"\n#if FIRST /*expect-E*/\n#endif\n'),
    ('cpp-scopes', 'cpp', ['-Werror=cpp'],
     '#pragma GCC diagnostic push\n#pragma GCC diagnostic ignored "-Wcpp"\n#warning hidden\n#pragma GCC diagnostic pop\n#warning visible /*expect-E*/\n'),
    ('file-macro-independent', 'missing-prototypes', ['-Wmissing-prototypes'],
     '#undef __FILE__\n#define __FILE__ 123\n#include "empty.h"\n_Pragma("GCC diagnostic push") _Pragma("GCC diagnostic ignored \\"-Wmissing-prototypes\\"") int f(void) { return 0; }\n_Pragma("GCC diagnostic pop") int g(void) { return 0; } /*expect-W*/\n'),
    ('line-control', 'missing-prototypes', ['-Wmissing-prototypes'],
     '#line 900 "presumed.c"\n_Pragma("GCC diagnostic push") _Pragma("GCC diagnostic ignored \\"-Wmissing-prototypes\\"") int f(void) { return 0; }\n_Pragma("GCC diagnostic pop") int g(void) { return 0; }\n'),
    ('repeated-include', 'strict-prototypes', ['-Wstrict-prototypes'],
     'int f(void);\n#pragma GCC diagnostic push\n#pragma GCC diagnostic ignored "-Wstrict-prototypes"\n#include "repeat.h"\n#pragma GCC diagnostic pop\n#include "repeat.h"\n'),
    ('pp-repeated-include', 'undef', ['-Wundef'],
     '#pragma GCC diagnostic push\n#pragma GCC diagnostic ignored "-Wundef"\n#include "undef.h"\n#pragma GCC diagnostic pop\n#include "undef.h"\n'),
    ('include-crossing-scope', 'missing-prototypes', ['-Wmissing-prototypes'],
     '#include "push.h"\nint f(void) { return 0; }\n#pragma GCC diagnostic pop\nint g(void) { return 0; } /*expect-W*/\n'),
    ('whitespace', 'missing-prototypes', ['-Wmissing-prototypes'],
     '# pragma\tGCC\t diagnostic   ignored   "-Wmissing-prototypes"\nint f(void) { return 0; }\n'),
]


def run(lccc: list[str], gcc: list[str]) -> None:
    version = subprocess.check_output(gcc + ['-dumpfullversion'], text=True).strip()
    if version != '16.2.0':
        raise RuntimeError(f'requires GCC 16.2.0, got {version}')
    checks = 0
    with tempfile.TemporaryDirectory(prefix='lccc-diagnostic-pragmas-') as td:
        root = Path(td)
        (root / 'undef.h').write_text('#if UNDEFINED\n#endif\n')
        (root / 'repeat.h').write_text('int f();\n')
        (root / 'push.h').write_text('#pragma GCC diagnostic push\n#pragma GCC diagnostic ignored "-Wmissing-prototypes"\n')
        (root / 'empty.h').write_text('/* Empty inclusion must not corrupt pragma provenance. */\n')
        for name, leaf, flags, source in CASES:
            expected = [(i, 'warning' if mark == 'W' else 'error')
                        for i, line in enumerate(source.splitlines(), 1)
                        for mark in re.findall(r'/\*expect-([WE])\*/', line)]
            if name in ('repeated-include', 'pp-repeated-include'):
                expected = [(1, 'warning')]
            if name == 'line-control':
                expected = [(901, 'warning')]
            path = root / f'{name}.c'
            path.write_text(source)
            modes = ['-fsyntax-only', '-c'] + (['-E'] if leaf in ('undef', 'cpp') else [])
            for mode in modes:
                for compiler_name, compiler in [('GCC', gcc), ('LCCC', lccc)]:
                    proc = subprocess.run(compiler + ['-std=gnu11', *flags, mode,
                        str(path), '-o', str(root / 'out')], capture_output=True, text=True, timeout=30)
                    actual = [(int(line), severity) for line, severity in re.findall(
                        r'^.*?:(\d+):(?:\d+:)?\s*(warning|error):[^\n]*\[-W(?:error=)?'
                        + re.escape(leaf) + r'\]', proc.stderr, re.M)]
                    error = any(level == 'error' for _, level in expected)
                    if actual != expected or bool(proc.returncode) != error:
                        raise RuntimeError(f'{name} {mode} {compiler_name}: expected {expected}, '
                            f'got {actual}, exit {proc.returncode}\n{proc.stderr}')
                checks += 1
            print(f'PASS diagnostic pragma {name}')
    print(f'diagnostic pragmas: PASS ({checks} mode checks against GCC {version})')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lccc', default='target/fastbuild/lccc')
    parser.add_argument('--gcc', default='gcc-16')
    args = parser.parse_args()
    run(shlex.split(args.lccc), shlex.split(args.gcc))
