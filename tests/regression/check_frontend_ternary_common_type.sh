#!/usr/bin/env bash
# The value of `?:` is a value of the operator's C common type (C11 6.5.15p5),
# and constant folding must not skip that conversion.
#
# Defect this pins: a constant condition made the fold return the live arm's
# constant untouched, at the arm's *storage* type. `1 ? -1 : 1u` then kept the
# sign of `-1`, so `(unsigned long long)(1 ? -1 : 1u)` printed
# 18446744073709551615 and `(long long)(...)` printed -1; gcc-16.2 and
# clang-23.1.4 both print 4294967295, because the conditional is an
# `unsigned int` whose value is 4294967295. The same fold feeds sema's constant
# values, so the defect was visible in `_Static_assert` and array bounds too.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
tmp=${TMPDIR:-/tmp}/lccc-ternary-common.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
cat >"$tmp/t.c" <<'C'
#define CHECK(expr, want) do { if ((long long)(expr) != (long long)(want)) return __LINE__; } while (0)
#define CHECKU(expr, want) do { if ((unsigned long long)(expr) != (unsigned long long)(want)) return __LINE__; } while (0)

/* Compile-time proof through the same constant evaluator. */
_Static_assert((1 ? -1 : 1u) == 4294967295u, "conditional value");
_Static_assert((0 ? -1 : 1u) == 1u, "unselected arm does not contribute");
_Static_assert((1 ? -1 : 1ull) == 0xffffffffffffffffull, "64-bit arm wins");
_Static_assert((int)(1 ? -1 : 1u) == -1, "converted back to int");
_Static_assert(sizeof(1 ? -1 : 1u) == 4, "width is unsigned int");
/* An ICE bound spelled with a folded conditional: (1 ? -1 : 1u) is truthy
   even though the -1 arm is negative, because the folded value is unsigned. */
static char bound[(1 ? -1 : 1u) ? 3 : 1];

int main(void)
{
    int cond = 1;
    /* Constant condition: folded. */
    CHECKU((unsigned long long)(1 ? -1 : 1u), 4294967295ull);
    CHECK((long long)(1 ? -1 : 1u), 4294967295ll);
    CHECKU((1 ? -1 : 1u), 4294967295ull);
    CHECKU((0 ? 1u : -1), 4294967295ull);
    /* Non-constant condition: merged through the branch path. */
    CHECKU((unsigned long long)(cond ? -1 : 1u), 4294967295ull);
    CHECK((long long)(cond ? -1 : 1u), 4294967295ll);
    /* Mixed widths and signedness either way. */
    CHECK((1 ? 5 : 6u), 5);
    CHECKU((1 ? 5u : -6), 5u);
    CHECK((1 ? -5L : 6u), -5);
    CHECKU((1 ? -5 : 6ul), 0xfffffffffffffffbull);
    CHECK((1 ? 7 : 8LL), 7);
    CHECK((1L ? (char)-1 : (short)-1), -1);
    /* GNU `a ?: b` shares the rule. */
    CHECKU((unsigned long long)(-1 ?: 1u), 4294967295ull);
    /* A pointer arm keeps its whole bit pattern. */
    int object = 0;
    CHECK((1 ? &object : (int *)0) == &object, 1);
    CHECK(sizeof bound, 3);
    return 0;
}
C
status=0
for level in -O0 -O1 -O2 -O3 -Os; do
    if ! "$CCC" "$level" "$tmp/t.c" -o "$tmp/t" >"$tmp/build.log" 2>&1; then
        echo "FAIL: $level did not compile" >&2
        cat "$tmp/build.log" >&2
        status=1
        continue
    fi
    rc=0
    "$tmp/t" || rc=$?
    if [ "$rc" -ne 0 ]; then
        echo "FAIL: $level exited $rc (that is the source line of the first mismatch)" >&2
        status=1
    fi
done
# The oracle must agree, so a change of mind here is a real regression and not a
# test that quietly encodes lccc's current behaviour.
if command -v gcc-16 >/dev/null 2>&1; then
    gcc-16 -O0 -w "$tmp/t.c" -o "$tmp/t.gcc"
    rc=0
    "$tmp/t.gcc" || rc=$?
    if [ "$rc" -ne 0 ]; then
        echo "FAIL: gcc-16 disagrees with the expectations (line $rc)" >&2
        status=1
    fi
fi
[ "$status" -eq 0 ] && echo "ternary-common-type: ok"
exit "$status"
