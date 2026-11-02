#!/usr/bin/env bash
# A `?:` is a value of the arms' C composite type (C11 6.5.15p5-6), and the IR
# carrier must be the one that type maps to in *every* context that consumes it:
# initializer, assignment, return, argument, an operand of another operator, a
# nested conditional, a cast.
#
# The three families below are the ones an integer-only test cannot reach, and
# the ones where `get_expr_type` used to report a type the lowering never
# produced (I32 for a D32/D64/U16 carrier):
#
#   * `_Decimal32`/`_Decimal64` -- GNU decimal floating point, supported by
#     GCC 16.2 and therefore measurable. Clang 23.1.4 rejects the types outright,
#     so GCC is the only oracle here and the gate says so.
#   * `_Float16` -- a 16-bit carrier that is neither an integer nor one of the
#     binary FP types the IR rank ladder knows.
#   * `_Complex float`/`_Complex double` -- lowered as a pointer to a
#     {real, imag} pair, so the value type is Ptr, not the integer width a
#     "common type" of two Ptrs would suggest.
#
# Each check prints the line number of the first mismatch and exits non-zero, so
# a failure names the property that broke instead of a diff.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
GCC=${GCC:-gcc-16}
tmp=${TMPDIR:-/tmp}/lccc-conditional-carrier.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
cat >"$tmp/c.c" <<'C'
#include <stdio.h>
#include <complex.h>

_Decimal64 d64(int c, _Decimal64 a, _Decimal64 b) { return c ? a : b; }
_Decimal64 d64_init(int c, _Decimal64 a, _Decimal64 b) { _Decimal64 r = c ? a : b; return r; }
_Decimal64 d64_add(int c, _Decimal64 a, _Decimal64 b) { return (c ? a : b) + 1.5dd; }
_Decimal64 d64_nested(int c, int d, _Decimal64 a, _Decimal64 b) { return c ? (d ? a : b) : (d ? b : a); }
int d64_cast(int c, _Decimal64 a, _Decimal64 b) { return (int)(c ? a : b); }
_Decimal32 d32(int c, _Decimal32 a, _Decimal32 b) { _Decimal32 r = c ? a : b; return r; }

_Float16 f16(int c, _Float16 a, _Float16 b) { return c ? a : b; }
_Float16 f16_init(int c, _Float16 a, _Float16 b) { _Float16 r = c ? a : b; return r; }
float f16_add(int c, _Float16 a, _Float16 b) { return (c ? a : b) + 1.0f; }
int f16_cast(int c, _Float16 a, _Float16 b) { return (int)(c ? a : b); }
_Float16 f16_mixed(int c, double a, _Float16 b) { return c ? a : b; }

_Complex double cx(int c, _Complex double a, _Complex double b) { return c ? a : b; }
_Complex double cx_init(int c, _Complex double a, _Complex double b) { _Complex double r = c ? a : b; return r; }
_Complex float cxf(int c, _Complex float a, _Complex float b) { return c ? a : b; }
double cx_arg(int c, _Complex double a, _Complex double b) { return creal(c ? a : b); }

int main(void)
{
    _Decimal64 a = 1.5dd, b = 2.5dd;
    _Decimal32 p = 3.5df, q = 9.5df;
    _Float16 h1 = 1.5f16, h2 = 2.5f16;
    _Complex double z1 = 1.0 + 2.0i, z2 = 3.0 + 4.0i;
    _Complex float w1 = 1.5f + 0.5fi, w2 = 2.5f + 1.5fi;
    int failures = 0;
#define CHECK(cond) do { if (!(cond)) { printf("line %d failed\n", __LINE__); failures++; } } while (0)
    CHECK(d64(1, a, b) == a);
    CHECK(d64(0, a, b) == b);
    CHECK(d64_init(0, a, b) == b);
    CHECK(d64_add(1, a, b) == a + 1.5dd);
    CHECK(d64_nested(1, 1, a, b) == a);
    CHECK(d64_nested(1, 0, a, b) == b);
    CHECK(d64_nested(0, 1, a, b) == b);
    CHECK(d64_cast(1, a, b) == 1);
    CHECK(d32(0, p, q) == q);
    CHECK((double)f16(1, h1, h2) == 1.5);
    CHECK((double)f16(0, h1, h2) == 2.5);
    CHECK((double)f16_init(0, h1, h2) == 2.5);
    CHECK(f16_add(0, h1, h2) == 3.5f);
    CHECK(f16_cast(1, h1, h2) == 1);
    CHECK((double)f16_mixed(0, 7.5, h2) == 2.5);
    CHECK((double)f16_mixed(1, 7.5, h2) == 7.5);
    CHECK(creal(cx(1, z1, z2)) == 1.0 && cimag(cx(1, z1, z2)) == 2.0);
    CHECK(creal(cx_init(0, z1, z2)) == 3.0 && cimag(cx_init(0, z1, z2)) == 4.0);
    CHECK(crealf(cxf(1, w1, w2)) == 1.5f && cimagf(cxf(1, w1, w2)) == 0.5f);
    CHECK(cx_arg(1, z1, z2) == 1.0);
    printf("%s\n", failures == 0 ? "all ok" : "failures");
    return failures == 0 ? 0 : 1;
}
C
status=0
for level in -O0 -O1 -O2 -O3 -Os; do
    if ! "$CCC" "$level" -w "$tmp/c.c" -o "$tmp/c" 2>"$tmp/build.log"; then
        echo "FAIL: $level did not compile" >&2
        head -5 "$tmp/build.log" >&2
        status=1
        continue
    fi
    out=$("$tmp/c" || true)
    if [ "$out" != "all ok" ]; then
        echo "FAIL: $level printed '$out' (that is the line of the first mismatch)" >&2
        status=1
    fi
done
# The oracle must agree with every check, so the expectations are GCC's too and
# not a description of what lccc happens to do today.
if command -v "$GCC" >/dev/null 2>&1; then
    if ! "$GCC" -O2 -w -std=gnu23 "$tmp/c.c" -o "$tmp/c.gcc" 2>"$tmp/gcc.log"; then
        echo "FAIL: the oracle rejected the case: $(head -3 "$tmp/gcc.log")" >&2
        status=1
    else
        out=$("$tmp/c.gcc" || true)
        [ "$out" = "all ok" ] || { echo "FAIL: $GCC disagrees: '$out'" >&2; status=1; }
    fi
fi
[ "$status" -eq 0 ] && echo "conditional-carrier: ok"
exit "$status"
