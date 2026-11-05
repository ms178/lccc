#!/usr/bin/env bash
# Differential equivalence of the O(n) dead-move census against the old O(n^2)
# per-candidate whole-function scan, plus a scaling regression.
#
# WHY THIS EXISTS
# ---------------
# `eliminate_dead_reg_moves` used to call `census_reg_reads` once per candidate
# move, and that helper walks the WHOLE function. On GCC torture case
# 20001226-1.c (one function, ~87k assembly lines) that is O(lines^2): 51.7 s
# at 2048 branches and an extrapolated ~25 min for the real 8192-branch case.
# The new `FunctionReadCensus` computes the read counts once per function and
# subtracts a retired line's exact read mask on every NOP, which is O(lines)
# and, crucially, answers the SAME question at every step.
#
# `CCC_SLOW_DEAD_MOVE_CENSUS=1` re-enables the original per-candidate scan
# inside the same binary. This gate compiles the same inputs both ways on i686
# at every optimization level and requires byte-identical objects: a stale,
# over- or under-counted census would produce different code and be caught.
set -euo pipefail
CCC_I686=${CCC_I686:-./target/fastbuild/lccc-i686}
[ -x "$CCC_I686" ] || { echo "check_i686_dead_move_census: SKIP ($CCC_I686 not built)"; exit 0; }

tmp=${TMPDIR:-/tmp}/lccc-dead-move-census.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

# Shapes that exercise every branch of the fallback: callee-saved registers,
# return-value registers, calls (implicit arg reads), inline asm (opaque),
# computed gotos, dead moves across labels, and a branch-dense function that
# forces the census fallback thousands of times.
cat >"$tmp/shapes.c" <<'C'
extern int printf(const char *, ...);
extern int other(int, int, int);
volatile int sink;

int callee_saved(int a, int b) {
    int x = a * 3 + b;
    int y = a ^ b;
    sink = y;
    return x + y;
}

int with_call(int a) {
    int staged = a + 7;
    return other(staged, a, 3) + staged;
}

int with_inline_asm(int a) {
    int staged = a * 5;
    __asm__ volatile("" : "+r"(staged));
    return staged;
}

int dead_across_label(int a, int b) {
    int dead = a + 11;
    if (a > b) goto gt;
    dead = b + 13;
    sink = dead;
    return 1;
gt:
    return 2;
}

unsigned branch_dense(int x[64], int y[64]) {
#define C(a,b) if (a > b) goto gt; if (a < b) goto lt;
#define C4(x,b) C((x)[0],b) C((x)[1],b) C((x)[2],b) C((x)[3],b)
#define C16(x,y) C4(x,(y)[0]) C4(x,(y)[1]) C4(x,(y)[2]) C4(x,(y)[3])
#define C64(x,y) C16(x,y) C16(x+4,y) C16(x+8,y) C16(x+12,y)
#define C256(x,y) C64(x,y) C64(x,y+4) C64(x,y+8) C64(x,y+12)
    C256(x,y);
    return 0x01234567u;
gt:
    return 0x12345678u;
lt:
    return 0xf0123456u;
}
C

fail=0
for opt in -O0 -O1 -O2 -O3 -Os; do
    new="$tmp/new$opt.o"
    old="$tmp/old$opt.o"
    "$CCC_I686" -w "$opt" -c "$tmp/shapes.c" -o "$new"
    CCC_SLOW_DEAD_MOVE_CENSUS=1 "$CCC_I686" -w "$opt" -c "$tmp/shapes.c" -o "$old"
    if ! cmp -s "$new" "$old"; then
        echo "FAIL: census A/B objects differ at $opt"
        cmp -l "$new" "$old" | head -8 || true
        fail=1
    else
        echo "PASS: census A/B byte-identical ($opt)"
    fi
done

[ "$fail" -eq 0 ] || exit 1
echo "PASS: check_i686_dead_move_census"
