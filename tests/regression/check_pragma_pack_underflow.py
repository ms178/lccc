#!/usr/bin/env python3
"""Exercise pack-stack underflow and its diagnostic policy.

The layout fixture uses only char and int; its expectations were measured
against GCC 16.2 for both x86 ABIs. Vendor references are independently
compiled with warnings suppressed because their pragma warning groups differ.
No target linker or runtime is needed.
"""

import argparse
from pathlib import Path
import subprocess
import tempfile


SOURCE = r'''#pragma pack(2)
#pragma pack(pop)
struct A { char c; int i; };
_Static_assert(sizeof(struct A) == 6, "unmatched pop changed current packing");
#pragma pack(push, 4)
#pragma pack(push, 1)
#pragma pack(pop)
struct B { char c; int i; };
_Static_assert(sizeof(struct B) == 8, "nested pop did not restore 4");
#pragma pack(pop)
#pragma pack(pop)
struct C { char c; int i; };
_Static_assert(sizeof(struct C) == 6, "second unmatched pop changed packing");
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wpragmas"
_Pragma("pack(pop)")
#pragma GCC diagnostic pop
struct D { char c; int i; };
_Static_assert(sizeof(struct D) == 6, "suppressed unmatched pop changed packing");
void f(void) {
#pragma pack(push, 1)
    struct E { char c; int i; };
    _Static_assert(sizeof(struct E) == 5, "block push did not take effect");
#pragma pack(pop)
#pragma pack(pop)
    struct F { char c; int i; };
    _Static_assert(sizeof(struct F) == 6, "block unmatched pop changed packing");
}
'''
# Marker-like identifiers are deliberately defined as macros to exercise
# blue-painting during rescan, not just straightforward operator expansion.
OPERATOR_SOURCE = r'''#define PUSH _Pragma("pack(push, 1)")
#define POP _Pragma("pack(pop)")
#define __ccc_pack_push_1 incorrect_macro_expansion
#define __ccc_pack_pop incorrect_macro_expansion
PUSH struct A { char c; int i; }; POP struct B { char c; int i; };
_Static_assert(sizeof(struct A) == 5, "operator push must affect A");
_Static_assert(sizeof(struct B) == 8, "operator pop must restore default");
#pragma pack(2)
PUSH struct C { char c; int i; }; POP struct D { char c; int i; };
_Static_assert(sizeof(struct C) == 5, "operator push must affect C");
_Static_assert(sizeof(struct D) == 6, "operator pop must restore 2");
#pragma pack()
void f(void) {
    PUSH struct E { char c; int i; }; POP struct F { char c; int i; };
    _Static_assert(sizeof(struct E) == 5, "block operator push");
    _Static_assert(sizeof(struct F) == 8, "block operator pop");
}
'''
MESSAGE = "'#pragma pack (pop)' encountered without matching '#pragma pack (push)'"


def run(command):
    result = subprocess.run(command, text=True, capture_output=True, timeout=30)
    return result.returncode, result.stdout, result.stderr


def require(condition, message, command, result):
    if not condition:
        raise SystemExit(f"FAIL: {message}\ncommand: {command!r}\n"
                         f"exit: {result[0]}\nstdout:\n{result[1]}\nstderr:\n{result[2]}")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--lccc', action='append', default=[], help='LCCC target executable; repeat for each target')
    parser.add_argument('--reference', action='append', default=[], help='reference compiler; repeat')
    parser.add_argument('--reference32', action='append', default=[], help='reference compiler with -m32; repeat')
    args = parser.parse_args()
    if not args.lccc and not args.reference and not args.reference32:
        parser.error('at least one compiler is required')
    with tempfile.TemporaryDirectory(prefix='lccc-pack-underflow-') as tmp:
        source = Path(tmp) / 'pack.c'
        source.write_text(SOURCE, encoding='utf-8')
        operator = Path(tmp) / 'operator.c'
        operator.write_text(OPERATOR_SOURCE, encoding='utf-8')
        for compiler, flags in ([(compiler, []) for compiler in args.reference] +
                                [(compiler, ['-m32']) for compiler in args.reference32]):
            # This is a layout/stack comparison, not a diagnostic parity claim.
            for fixture in (source, operator):
                command = [compiler, *flags, '-w', '-fsyntax-only', str(fixture)]
                result = run(command)
                require(result[0] == 0, 'reference layout', command, result)
            print(f'PASS reference layout: {compiler} {flags}', flush=True)
        # Replace the first unmatched pop with a reset. This models the old
        # state transition at the fixture boundary, not a mutated compiler.
        mutant = Path(tmp) / 'mutated.c'
        mutant.write_text(SOURCE.replace('#pragma pack(pop)', '#pragma pack(0)', 1), encoding='utf-8')
        for compiler in args.lccc:
            def check(flags, expected, count=None, preprocess=False):
                command = [compiler, *flags, str(source)]
                result = run(command)
                require((result[0] == 0) == expected, 'exit status', command, result)
                if count is not None:
                    require(result[2].count(MESSAGE) == count, 'diagnostic count', command, result)
                if preprocess:
                    require(not result[2], 'preprocessing emitted diagnostics', command, result)
                    require('#pragma pack(pop)' in result[1] and '__ccc_pack_' not in result[1],
                            'preprocessed pragmas were lost or internal markers leaked', command, result)
            check(['-fsyntax-only'], True, 3)
            check(['-fsyntax-only', '-Werror=pragmas'], False, 3)
            check(['-fsyntax-only', '-Werror', '-Wno-pragmas'], True, 0)
            check(['-E', '-Werror=pragmas'], True, 0, preprocess=True)
            check(['-c', '-Wno-pragmas', '-o', str(Path(tmp) / 'pack.o')], True, 0)
            command = [compiler, '-fsyntax-only', '-Wno-pragmas', str(mutant)]
            result = run(command)
            require(result[0] != 0, 'reset-on-underflow fixture mutation was not detected', command, result)
            command = [compiler, '-fsyntax-only', '-Werror', str(operator)]
            result = run(command)
            require(result[0] == 0, 'operator layout', command, result)
            command = [compiler, '-E', str(operator)]
            result = run(command)
            require(result[0] == 0 and '__ccc_pack_' not in result[1],
                    'operator preprocessing leaked markers', command, result)
            preprocessed = Path(tmp) / 'operator.i'
            preprocessed.write_text(result[1], encoding='utf-8')
            command = [compiler, '-fsyntax-only', '-Werror', str(preprocessed)]
            result = run(command)
            require(result[0] == 0, 'operator preprocessed round trip', command, result)
            print(f'PASS LCCC layout, diagnostics, operator and fixture mutation: {compiler}', flush=True)


if __name__ == '__main__':
    main()
