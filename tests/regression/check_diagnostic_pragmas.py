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
    ('pragma-message-scopes', 'pragmas', ['-Werror=pragmas'],
     '#pragma GCC diagnostic push\n#pragma GCC diagnostic ignored "-Wpragmas"\n#pragma message 1\n#pragma GCC diagnostic pop\n#pragma message 2 /*expect-E*/\n'),
    ('pragma-message-demote', 'pragmas', ['-Werror=pragmas'],
     '#pragma GCC diagnostic warning "-Wpragmas"\n#pragma message 1 /*expect-W*/\n'),
    ('pragma-message-promote', 'pragmas', ['-Wno-pragmas'],
     '#pragma GCC diagnostic error "-Wpragmas"\n#pragma message 1 /*expect-E*/\n'),
    ('cpp-scopes', 'cpp', ['-Werror=cpp'],
     '#pragma GCC diagnostic push\n#pragma GCC diagnostic ignored "-Wcpp"\n#warning hidden\n#pragma GCC diagnostic pop\n#warning visible /*expect-E*/\n'),
    ('file-macro-independent', 'missing-prototypes', ['-Wmissing-prototypes'],
     '#undef __FILE__\n#define __FILE__ 123\n#include "empty.h"\n_Pragma("GCC diagnostic push") _Pragma("GCC diagnostic ignored \\"-Wmissing-prototypes\\"") int f(void) { return 0; }\n_Pragma("GCC diagnostic pop") int g(void) { return 0; } /*expect-W*/\n'),
    ('line-control', 'missing-prototypes', ['-Wmissing-prototypes'],
     '#line 900 "presumed.c"\n_Pragma("GCC diagnostic push") _Pragma("GCC diagnostic ignored \\"-Wmissing-prototypes\\"") int f(void) { return 0; }\n_Pragma("GCC diagnostic pop") int g(void) { return 0; }\n'),
    ('line-control-file-macro', 'missing-prototypes', ['-Wmissing-prototypes'],
     '#line 900 "presumed.c"\n#undef __FILE__\n#define __FILE__ 123\n#pragma GCC diagnostic push\n#pragma GCC diagnostic ignored "-Wmissing-prototypes"\nint f(void) { return 0; }\n#pragma GCC diagnostic pop\nint g(void) { return 0; }\n'),
    ('repeated-include', 'strict-prototypes', ['-Wstrict-prototypes'],
     'int f(void);\n#pragma GCC diagnostic push\n#pragma GCC diagnostic ignored "-Wstrict-prototypes"\n#include "repeat.h"\n#pragma GCC diagnostic pop\n#include "repeat.h"\n'),
    ('pp-repeated-include', 'undef', ['-Wundef'],
     '#pragma GCC diagnostic push\n#pragma GCC diagnostic ignored "-Wundef"\n#include "undef.h"\n#pragma GCC diagnostic pop\n#include "undef.h"\n'),
    ('include-crossing-scope', 'missing-prototypes', ['-Wmissing-prototypes'],
     '#include "push.h"\nint f(void) { return 0; }\n#pragma GCC diagnostic pop\nint g(void) { return 0; } /*expect-W*/\n'),
    ('whitespace', 'missing-prototypes', ['-Wmissing-prototypes'],
     '# pragma\tGCC\t diagnostic   ignored   "-Wmissing-prototypes"\nint f(void) { return 0; }\n'),
]


def check_message_scope_composition(root: Path, lccc: list[str], gcc: list[str]) -> int:
    """Both sides of the merge must survive together on every driver path."""
    source = root / 'pragma-composition.c'
    source.write_text('''#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wundef"
#if FIRST
#endif
#pragma GCC warning "merge-marker"
#pragma message "merge-note é"
#pragma GCC diagnostic pop
#if SECOND
#endif
#if u'A' != 65 || L'A' != 65 || 'x' != 120
#error character-token-regression
#endif
int f(void) { return 0; }
''')
    checks = 0
    for flags in (['-Wundef'], ['-Wundef', '-Wno-cpp'], ['-Werror=undef'],
                  ['-Werror', '-Wundef'], ['-Werror', '-Wundef', '-Wno-error=undef'],
                  ['-w', '-Werror']):
        for mode in ('-E', '-fsyntax-only', '-c'):
            mute = '-w' in flags
            undef = ('error' if '-Werror=undef' in flags or
                     ('-Werror' in flags and '-Wno-error=undef' not in flags)
                     else 'warning')
            # The mixed blanket-error/demotion case must explicitly select undef.
            selected = '-Wundef' in flags or '-Werror=undef' in flags
            expected = ([] if mute else [(5, 'error' if '-Werror' in flags else 'warning', 'merge-marker')])
            if mode != '-E':
                expected += [(6, 'note', 'merge-note')]
            if selected and not mute:
                expected += [(8, undef, 'SECOND')]
            expected.sort()
            error = any(level == 'error' for _, level, _ in expected)
            for compiler in [gcc] + [[lccc[0] + suffix, *lccc[1:]]
                                      for suffix in ('', '-i686', '-arm', '-riscv')]:
                proc = subprocess.run(compiler + ['-std=gnu11', *flags, mode, str(source),
                    '-o', str(root / 'composition-out')], capture_output=True, text=True, timeout=30)
                found = []
                for line, level, text in re.findall(
                        r'^.*?:(\d+):(?:\d+:)?\s*(warning|error|note):([^\n]*)', proc.stderr, re.M):
                    marker = next((m for m in ('merge-marker', 'merge-note', 'SECOND') if m in text), None)
                    if marker:
                        found.append((int(line), level, marker))
                if sorted(found) != expected or bool(proc.returncode) != error:
                    raise RuntimeError(f'pragma composition {compiler} {mode} {flags}: '
                        f'{sorted(found)} != {expected}, exit {proc.returncode}\n{proc.stderr}')
                # No other diagnostic may conceal an undeclared merge regression.
                relevant = re.findall(r'^.*?:\d+:(?:\d+:)?\s*(?:warning|error|note):([^\n]*)', proc.stderr, re.M)
                if any(not any(m in text for m in ('merge-marker', 'merge-note', 'SECOND')) for text in relevant):
                    raise RuntimeError(f'unexpected composition diagnostic: {compiler}\n{proc.stderr}')
            checks += 4
    return checks


def check_message_preprocess_roundtrip(root: Path, lccc: list[str], gcc: list[str]) -> int:
    source = root / 'message-roundtrip.c'
    source.write_text('''#define S_(x) #x
#define S(x) S_(x)
#define P_(x) _Pragma(#x)
#define P(x) P_(x)
#define ALIAS _Pragma
#pragma message ("direct  é")
P(message("macro  é"))
ALIAS("message(\\\"rescan  é\\\")")
P(message(S(__COUNTER__)))
int count_value[__COUNTER__ == 1 ? 1 : -1];
''')
    compilers = [gcc] + [[lccc[0] + suffix, *lccc[1:]]
                         for suffix in ('', '-i686', '-arm', '-riscv')]
    checks = 0
    for i, producer in enumerate(compilers):
        output = root / f'message-roundtrip-{i}.c'
        proc = subprocess.run(producer + ['-std=gnu11', '-E', '-P', str(source), '-o', str(output)],
                              capture_output=True, text=True, timeout=30)
        if proc.returncode or proc.stderr:
            raise RuntimeError(f'message preprocessing must defer all diagnostics: {producer}\n{proc.stderr}')
        text = output.read_text()
        if len(re.findall(r'^\s*#\s*pragma\s+message\b', text, re.M)) != 4:
            raise RuntimeError(f'missing deferred messages: {producer}\n{text}')
        if 'direct  é' not in text or 'macro  é' not in text:
            raise RuntimeError(f'quoted whitespace/UTF-8 was not preserved: {producer}\n{text}')
        for consumer in compilers:
            proc = subprocess.run(consumer + ['-std=gnu11', '-fsyntax-only', str(output)],
                                  capture_output=True, text=True, timeout=30)
            messages = re.findall(r"note: '#pragma message: ([^\n]*)'", proc.stderr)
            if proc.returncode or messages != ['direct  é', 'macro  é', 'rescan  é', '0']:
                raise RuntimeError(f'roundtrip {producer} -> {consumer}: {messages}\n{proc.stderr}\n{text}')
            checks += 1
    # A self-referential operand is expanded once, not again when a queued
    # _Pragma effect is drained. The array makes extra __COUNTER__ uses fail.
    recursive = root / 'message-selfref.c'
    recursive.write_text('#define SELF SELF __COUNTER__\n_Pragma("message SELF")\nint counter_bound[__COUNTER__ == 1 ? 1 : -1];\n')
    for i, producer in enumerate(compilers):
        output = root / f'message-selfref-{i}.c'
        proc = subprocess.run(producer + ['-E', '-P', str(recursive), '-o', str(output)],
                              capture_output=True, text=True, timeout=30)
        if proc.returncode or proc.stderr:
            raise RuntimeError(f'self-referential message preprocessing: {producer}\n{proc.stderr}')
        for consumer in compilers:
            proc = subprocess.run(consumer + ['-fsyntax-only', str(output)],
                                  capture_output=True, text=True, timeout=30)
            warnings = re.findall(r'warning:[^\n]*expected a string[^\n]*\[-Wpragmas\]', proc.stderr)
            if proc.returncode or len(warnings) != 1 or 'error:' in proc.stderr:
                raise RuntimeError(f'self-referential roundtrip {producer} -> {consumer}\n{proc.stderr}')
            checks += 1
    # Assembly -E has no C frontend: libcpp warnings still report, but
    # message operands expand once and their frontend directives are dropped.
    assembly = root / 'message-roundtrip.S'
    assembly.write_text('#warning cpp-asm-merge\n#pragma GCC warning "pragma-asm-merge"\n#pragma message __COUNTER__\n.long __COUNTER__\nnop\n')
    for flags, expected in [([], ['warning', 'warning']),
                            (['-Wno-cpp'], ['warning']),
                            (['-Werror'], ['error', 'error']),
                            (['-w', '-Werror'], [])]:
        for compiler in compilers:
            proc = subprocess.run(compiler + ['-E', '-P', *flags, str(assembly)],
                                  capture_output=True, text=True, timeout=30)
            levels = re.findall(r'^.*?:\d+:(?:\d+:)?\s*(warning|error):[^\n]*(?:cpp-asm-merge|pragma-asm-merge)',
                                proc.stderr, re.M)
            if levels != expected or bool(proc.returncode) != ('error' in expected):
                raise RuntimeError(f'assembly -E diagnostics {compiler} {flags}: {levels} != {expected}\n{proc.stderr}')
            if ('#pragma message' in proc.stdout or 'expected a string' in proc.stderr
                    or not re.search(r'^\.long\s+1\s*$', proc.stdout, re.M)):
                raise RuntimeError(f'assembly -E handled a C-frontend pragma: {compiler}\n{proc.stdout}\n{proc.stderr}')
        checks += 4
    return checks


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
            if name == 'line-control-file-macro':
                expected = [(906, 'warning')]
            if name == 'line-control':
                expected = [(901, 'warning')]
            path = root / f'{name}.c'
            path.write_text(source)
            modes = ['-fsyntax-only', '-c'] + (['-E'] if leaf in ('undef', 'cpp', 'pragmas') else [])
            for mode in modes:
                mode_expected = [] if leaf == 'pragmas' and mode == '-E' else expected
                compilers = [('GCC', gcc)] + [
                    ('LCCC' + suffix, [lccc[0] + suffix, *lccc[1:]])
                    for suffix in ('', '-i686', '-arm', '-riscv')]
                for compiler_name, compiler in compilers:
                    proc = subprocess.run(compiler + ['-std=gnu11', *flags, mode,
                        str(path), '-o', str(root / 'out')], capture_output=True, text=True, timeout=30)
                    actual = [(int(line), severity) for line, severity in re.findall(
                        r'^.*?:(\d+):(?:\d+:)?\s*(warning|error):[^\n]*\[-W(?:error=)?'
                        + re.escape(leaf) + r'\]', proc.stderr, re.M)]
                    error = any(level == 'error' for _, level in mode_expected)
                    if actual != mode_expected or bool(proc.returncode) != error:
                        raise RuntimeError(f'{name} {mode} {compiler_name}: expected {mode_expected}, '
                            f'got {actual}, exit {proc.returncode}\n{proc.stderr}')
                checks += 4
            print(f'PASS diagnostic pragma {name}')
        checks += check_message_scope_composition(root, lccc, gcc)
        checks += check_message_preprocess_roundtrip(root, lccc, gcc)
    print(f'diagnostic pragmas: PASS ({checks} four-target mode checks against GCC {version})')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lccc', default='target/fastbuild/lccc')
    parser.add_argument('--gcc', default='gcc-16')
    args = parser.parse_args()
    run(shlex.split(args.lccc), shlex.split(args.gcc))
