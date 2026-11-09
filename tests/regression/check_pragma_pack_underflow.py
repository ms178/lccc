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
    args = parser.parse_args()
    if not args.lccc and not args.reference:
        parser.error('at least one compiler is required')
    with tempfile.TemporaryDirectory(prefix='lccc-pack-underflow-') as tmp:
        source = Path(tmp) / 'pack.c'
        source.write_text(SOURCE, encoding='utf-8')
        for compiler in args.reference:
            # This is a layout/stack comparison, not a diagnostic parity claim.
            command = [compiler, '-w', '-fsyntax-only', str(source)]
            result = run(command)
            require(result[0] == 0, 'reference layout', command, result)
            print(f'PASS reference layout: {compiler}', flush=True)
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
            print(f'PASS LCCC layout and diagnostics: {compiler}', flush=True)


if __name__ == '__main__':
    main()
