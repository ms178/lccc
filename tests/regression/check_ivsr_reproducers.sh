#!/usr/bin/env bash
# AUD-HYGIENE-1: the IVSR/univsr probe sources that used to sit loose in
# `tests/` (simple_loop.c, test_ivsr_indexed.c, plus their pre-fix `.s`
# snapshots) are gates now.
#
# The probes only make sense as a CONTRACT, because their interesting property
# is not "which instructions the pass happens to pick today" -- that changes
# with every improvement -- but that turning induction-variable strength
# reduction on or off cannot change what the program computes, on the shapes the
# pass targets: a plain indexed sum, a sum through a struct member, an indexed
# fill, and an indexed read-modify-write.
#
# So the gate is metamorphic in the pass and differential against GCC:
#
#   lccc -O2                      (pass on)   ==  GCC -O2
#   lccc -O2 CCC_NO_IVSR=1        (pass off)  ==  the leg above
#   lccc -O3                      (vectorizer on top)  still the same numbers
#
# A wrong result here is a miscompile in the loop-strength-reduction or
# vectorization path, not an instruction-selection quibble.
#
# Usage: CCC=target/fastbuild/lccc bash tests/regression/check_ivsr_reproducers.sh
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
ccc=${CCC:-$repo/target/fastbuild/lccc}
[ -x "$ccc" ] || { echo "SKIP: $ccc not built" >&2; exit 0; }

src=$repo/tests/regression/ivsr-probes/ivsr_indexed_loop.c
simple=$repo/tests/regression/ivsr-probes/ivsr_simple_loop.c
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

# The driver is generated here, not checked in: it must stay in lockstep with
# the probe signatures above it, and a mismatch is a compile error rather than
# a silently skipped comparison.
cat >"$td/driver.c" <<'EOF'
#include <stdio.h>

/* Declared BEFORE the prototype on purpose: a `struct Point` first mentioned
   inside a parameter list has function-prototype scope, i.e. it is a different
   type from the file-scope one, and the two TUs then disagree about how the
   argument is passed. */
struct Point { double x, y, z; };  /* must match the probe TU, 24 bytes */

double sum_array(const double *arr, int n);
double sum_points_x(const struct Point *pts, int n);
void fill_array(double *arr, int n, double value);
void scale_array(double *arr, int n, double factor);
double test(const double *arr, int n);

static double arr[257];

int main(void) {
    for (int i = 0; i < 257; i++) {
        arr[i] = (double)(i % 17) * 0.5 - 3.25;
    }
    /* Odd trip counts on purpose: a strength-reduction prologue/epilogue that
       is off by one shows up here and nowhere in a power-of-two run. */
    for (int n = 0; n <= 257; n += 7) {
        printf("sum %d %.6f\n", n, sum_array(arr, n));
        printf("test %d %.6f\n", n, test(arr, n));

        struct Point pts[40];
        for (int i = 0; i < 40; i++) {
            pts[i].x = arr[(i * 3) % 257];
            pts[i].y = arr[(i * 5 + 1) % 257];
            pts[i].z = arr[(i * 7 + 2) % 257];
        }
        printf("pts %d %.6f\n", n, sum_points_x(pts, n < 40 ? n : 40));

        double buf[64];
        for (int i = 0; i < 64; i++) {
            buf[i] = 1.0;
        }
        fill_array(buf, n < 64 ? n : 64, 2.5);
        for (int i = 0; i < 64; i++) {
            buf[i] += (double)i;
        }
        scale_array(buf, n < 64 ? n : 64, -0.75);
        double acc = 0.0;
        for (int i = 0; i < 64; i++) {
            acc += buf[i] * (double)(i + 1);
        }
        printf("buf %d %.6f\n", n, acc);
    }
    return 0;
}
EOF

gcc -O2 -fno-strict-aliasing "$td/driver.c" "$src" "$simple" -o "$td/gcc" 2>"$td/gcc.err" ||
    { cat "$td/gcc.err" >&2; fail "gcc could not build the probes"; }
"$td/gcc" >"$td/gcc.out" || fail "gcc reference exited non-zero"

# 37 trip counts (0,7,...,252) x 4 shapes; an exact count, so a truncated or
# aborted sweep cannot pass by producing "enough" output.
lines=$(wc -l <"$td/gcc.out")
[ "$lines" -eq 148 ] || fail "the driver produced $lines lines; expected 148 (37 trip counts x 4 shapes)"

for opt in -O2 -O3; do
    tag="on${opt#-}"
    "$ccc" "$opt" "$td/driver.c" "$src" "$simple" -o "$td/$tag" 2>"$td/$tag.err" ||
        { cat "$td/$tag.err" >&2; fail "$tag: compile failed"; }
    "$td/$tag" >"$td/$tag.out" || fail "$tag: program exited non-zero"
    cmp -s "$td/gcc.out" "$td/$tag.out" ||
        { diff -u "$td/gcc.out" "$td/$tag.out" | sed -n '1,20p' >&2;
          fail "$opt ($tag): IVSR-primed loop disagreed with GCC"; }
done

for opt in -O0 -O1 -O2 -O3; do
    tag="off${opt#-}"
    CCC_NO_IVSR=1 "$ccc" "$opt" "$td/driver.c" "$src" "$simple" -o "$td/$tag" 2>"$td/$tag.err" ||
        { cat "$td/$tag.err" >&2; fail "$tag: compile failed"; }
    "$td/$tag" >"$td/$tag.out" || fail "$tag: program exited non-zero"
    cmp -s "$td/gcc.out" "$td/$tag.out" ||
        { diff -u "$td/gcc.out" "$td/$tag.out" | sed -n '1,20p' >&2;
          fail "$opt ($tag): CCC_NO_IVSR=1 disagreed with GCC"; }
done

echo "PASS: IVSR/univsr probe shapes agree with GCC with and without the pass"
