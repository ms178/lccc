#!/usr/bin/env bash
# _Static_assert with type-dependent conditions: the positive fixture must
# compile, and every false assertion below must be rejected. These used to
# pass silently because the parser accepted any condition it could not
# evaluate without types. Each negative case is its own throwaway TU (the
# regression corpus compiles every checked-in C file as a positive case) and
# is checked against the host C compiler too, so a wrong expectation fails.
set -euo pipefail

root=$(cd -- "$(dirname -- "$0")/../.." && pwd)
CCC=${CCC:-"$root/target/fastbuild/lccc"}
CC=${CC:-gcc}
[[ -x "$CCC" ]] || { echo "missing LCCC compiler: $CCC" >&2; exit 1; }
command -v "$CC" >/dev/null 2>&1 || { echo "missing C oracle: $CC" >&2; exit 1; }
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

for opt in -O0 -O2; do
    "$CC" -std=gnu17 "$opt" -c "$root/tests/regression/static_assert_sema.c" -o "$work/oracle.o"
    "$CCC" "$opt" -c "$root/tests/regression/static_assert_sema.c" -o "$work/lccc.o"
done

prelude='#include <stddef.h>
#define TY(x) _Generic((x), int: 1, unsigned: 2, default: 0)
enum Small { SA };
struct S { int a; char b; };'
negatives=(
    '_Static_assert(TY(SA) == 2, "generic");'
    '_Static_assert(sizeof(SA) == 8, "enumerator sizeof");'
    '_Static_assert(offsetof(struct S, b) == 2, "offsetof");'
    '_Static_assert(sizeof(struct S) == 99);'
    'void f(void) { enum { L = 0x80000000u }; _Static_assert(TY(L) == 1, "block"); }'
    'void f(void) { { enum Small { SV = -5 }; } _Static_assert(TY(SA) == 1 && sizeof(enum Small) == 8, "scope"); }'
    'struct M { int x; _Static_assert(sizeof(struct S) == 3, "member"); };'
    'void f(void) { struct N { _Static_assert(_Alignof(struct S) == 1, "nested member"); int y; } n; (void)n; }'
)
for i in "${!negatives[@]}"; do
    src="$work/neg$i.c"
    printf '%s\n%s\n' "$prelude" "${negatives[$i]}" >"$src"
    if "$CC" -std=gnu17 -c "$src" -o "$work/neg.o" 2>/dev/null; then
        echo "oracle $CC accepted negative case $i: ${negatives[$i]}" >&2
        exit 1
    fi
    if "$CCC" -c "$src" -o "$work/neg.o" >"$work/neg$i.err" 2>&1; then
        echo "FAIL: lccc accepted a false static assertion: ${negatives[$i]}" >&2
        exit 1
    fi
    grep -q 'static assertion failed' "$work/neg$i.err" || {
        echo "FAIL: case $i rejected without the static-assertion diagnostic" >&2
        cat "$work/neg$i.err" >&2
        exit 1
    }
done
echo "static assert sema: PASS (positive fixture at -O0/-O2, ${#negatives[@]} false assertions rejected)"
