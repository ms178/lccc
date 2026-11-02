#!/usr/bin/env bash
# The `-fno-delete-null-pointer-checks` contract (kernel builds pass it
# unconditionally):
#
#   1. LCCC never infers non-nullness of a pointer from having dereferenced
#      it — in ANY optimisation mode and with or without the flag. The nine
#      shapes below are the canonical GCC-deletion probes (same-block,
#      phi-merged, loop-carried, then-arm-join, field/base, store-through,
#      both-arm GVN, call-clobbered, low-address range): each dereferences a
#      pointer and then tests it again. GCC's default `-fdelete-null-pointer-
#      checks` folds those later tests away (measured: f1/f5/f6/f8 lose their
#      guard entirely on GCC default and keep it under the flag), while LCCC
#      must keep every guard at -O2 and -O3 — the guard ratchet below pins a
#      per-shape minimum of test/cmp instructions (counting tests, not
#      branches, so a legitimate if-convert to cmov/sete keeps the gate green
#      while a deleted guard cannot hide).
#   2. The flag itself is an *off-request*: the deletion it disables does not
#      exist here, so LCCC accepts it SILENTLY in normal and strict mode (a
#      warning would tell every translation unit something false). Assembly
#      with and without the flag must be byte-identical.
#   3. The positive `-fdelete-null-pointer-checks` asks to DELETE checks,
#      which nothing here does: it stays on the tolerated path — exit 0 with
#      a naming warning in normal mode, refusal under LCCC_STRICT_OPTIONS=1.
#   4. The GCC differential also asserts the fixture is *sensitive*: GCC
#      default must measure strictly fewer guards than GCC -fno-delete
#      (otherwise a future GCC could make this ratchet vacuous).
#
# Usage:  env CCC=... [GCC=gcc-16] bash tests/regression/check_nullptr_check_policy.sh
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
CCC=${CCC:-$root/target/fastbuild/lccc}
GCC=${GCC:-gcc-16}
command -v "$CCC" >/dev/null || { echo "missing lccc: $CCC" >&2; exit 1; }
# A development box with only the distro GCC still gets a usable gate; CI pins
# GCC 16.2 through $GCC, and the differential is only as good as the oracle.
if ! command -v "$GCC" >/dev/null; then GCC=gcc; fi
command -v "$GCC" >/dev/null || { echo "missing oracle: $GCC" >&2; exit 1; }
tmp=$(mktemp -d)
trap 'rm -rf -- "$tmp"' EXIT

cat > "$tmp/shapes.c" <<'EOF'
/* Nine canonical -fno-delete-null-pointer-checks shapes: each dereferences
 * a pointer and then tests it (or its alias) again. */
struct W { int a; };
extern void side(void);
/* f1: same-block deref then test */
int f1(int *p, int n) { int v = *p; if (p == 0) return -1; return v + n; }
/* f2: both paths deref, phi-merged test */
int f2(int *p, int *q, int c) {
    int a = *p; int *m = c ? p : q; int b = *m;
    if (m == 0) return a; return a + b;
}
/* f3: loop-carried guard after deref */
int f3(int **pp, int n) {
    int s = 0;
    for (int i = 0; i < n; i++) { int *p = pp[i]; s += *p; if (p == 0) break; }
    return s;
}
/* f4: deref in then-arm, test after the join */
int f4(int *p, int c) { int r = 0; if (c) { r = *p; } if (p == 0) r += 7; return r; }
/* f5: field deref of base, then base test */
int f5(struct W *b, int n) { int x = b->a; if (b == 0) return n; return x; }
/* f6: store through p, test after */
int f6(int *p, int v) { *p = v; if (!p) return 1; return 0; }
/* f7: both arms deref, GVN-merged, test at join */
int f7(int *p, int *q, int c) {
    if (c) { *p = 1; } else { *q = 2; }
    int *m = c ? p : q;
    if (m == 0) return -9; return *m;
}
/* f8: deref, call clobber, test after call */
int f8(int *p) { int v = *p; side(); if (p == 0) return -2; return v; }
/* f9: low-address range test after deref */
int f9(long p) { int *q = (int *)p; int v = *q; if ((unsigned long)q < 4096) return -3; return v; }
EOF

printf 'int probe(void) { return 7; }\n' > "$tmp/probe.c"

# ---- 1. acceptance / warning contract --------------------------------------
# The off-request is silent in normal AND strict mode; the positive form is
# diagnosed (tolerated) normally and refused strictly.
err=$("$CCC" -O2 -fno-delete-null-pointer-checks -c "$tmp/probe.c" -o "$tmp/p1.o" 2>&1) || {
    echo "FAIL: -fno-delete-null-pointer-checks exited nonzero" >&2; echo "$err" >&2; exit 1; }
if grep -q "tolerated but not implemented" <<<"$err"; then
    echo "FAIL: the off-request must be accepted silently (state is already the state)" >&2
    echo "$err" >&2; exit 1
fi
LCCC_STRICT_OPTIONS=1 "$CCC" -O2 -fno-delete-null-pointer-checks -c "$tmp/probe.c" -o "$tmp/p2.o" >/dev/null 2>&1 || {
    echo "FAIL: strict mode must accept -fno-delete-null-pointer-checks (request is met)" >&2; exit 1; }
err=$("$CCC" -O2 -fdelete-null-pointer-checks -c "$tmp/probe.c" -o "$tmp/p3.o" 2>&1) || {
    echo "FAIL: positive spelling must tolerate (exit 0) in normal mode" >&2; echo "$err" >&2; exit 1; }
grep -Fq -- "-fdelete-null-pointer-checks" <<<"$err" || {
    echo "FAIL: positive spelling must be diagnosed by name" >&2; echo "$err" >&2; exit 1; }
if LCCC_STRICT_OPTIONS=1 "$CCC" -O2 -fdelete-null-pointer-checks -c "$tmp/probe.c" -o "$tmp/p4.o" >/dev/null 2>&1; then
    echo "FAIL: strict mode must refuse the positive spelling (nothing will delete checks)" >&2; exit 1
fi

# ---- 2. the flag changes nothing: byte-identical assembly -------------------
for O in -O2 -O3; do
    "$CCC" $O -S "$tmp/shapes.c" -o "$tmp/base_$O.s" 2>/dev/null
    "$CCC" $O -fno-delete-null-pointer-checks -S "$tmp/shapes.c" -o "$tmp/flag_$O.s" 2>/dev/null
    cmp -s "$tmp/base_$O.s" "$tmp/flag_$O.s" || {
        echo "FAIL: -fno-delete-null-pointer-checks changed $O assembly (must be inert)" >&2
        diff -u "$tmp/base_$O.s" "$tmp/flag_$O.s" | sed -n '1,40p' >&2; exit 1; }
done

# ---- 3. guard ratchet: every shape keeps its test/cmp -----------------------
# Minimums are the measured 2026-10-31 values at both -O2 and -O3
# (f1=1 f2=2 f3=3 f4=2 f5=1 f6=1 f7=3 f8=1 f9=1). A drop below any minimum
# means a null-guard comparison disappeared — a deletion-CLASS regression.
# Counting tests rather than branches keeps legitimate if-conversion (test +
# cmov/sete) green while a deleted guard cannot hide.
guard_count() { # guard_count <asm-file> <fn>
    awk -v fn="$2" '$0==fn":"{on=1} on&&$0==".size "fn", .-"fn{on=0} on' "$1" |
        grep -cE '^[[:space:]]*(test|cmp)[bwlq]?[[:space:]]' || true
}
declare -A MIN=( [f1]=1 [f2]=2 [f3]=3 [f4]=2 [f5]=1 [f6]=1 [f7]=3 [f8]=1 [f9]=1 )
for O in -O2 -O3; do
    for f in f1 f2 f3 f4 f5 f6 f7 f8 f9; do
        n=$(guard_count "$tmp/base_$O.s" "$f")
        if (( n < MIN[$f] )); then
            echo "FAIL: LCCC $O $f keeps $n guard test(s), minimum ${MIN[$f]} — a null check was deleted" >&2
            exit 1
        fi
    done
done

# ---- 4. GCC differential: the fixture is sensitive, and both sides keep it --
# GCC default deletes (f1 drops to zero guards); GCC -fno-delete must keep it.
# The total must strictly decrease from -fno-delete to default, so a future
# GCC that stops deleting cannot make the ratchet vacuous unnoticed.
gcc_guards() { # gcc_guards <asm-file> <fn>
    awk -v fn="$2" '$0==fn":"{on=1} on&&$0=="\t.size\t"fn", .-"fn{on=0} on' "$1" |
        grep -cE '^[[:space:]]*(test|cmp)[bwlq]?[[:space:]]' || true
}
"$GCC" -O2 -fno-delete-null-pointer-checks -S "$tmp/shapes.c" -o "$tmp/gcc_nd.s" 2>/dev/null
"$GCC" -O2 -S "$tmp/shapes.c" -o "$tmp/gcc_def.s" 2>/dev/null
def_total=0 nd_total=0
for f in f1 f2 f3 f4 f5 f6 f7 f8 f9; do
    d=$(gcc_guards "$tmp/gcc_def.s" "$f")
    n=$(gcc_guards "$tmp/gcc_nd.s" "$f")
    def_total=$((def_total + d)); nd_total=$((nd_total + n))
done
(( nd_total > def_total )) || {
    echo "FAIL: GCC sensitivity probe broke (default=$def_total no-delete=$nd_total); fixture no longer detects deletion" >&2
    exit 1; }
f1_nd=$(gcc_guards "$tmp/gcc_nd.s" f1)
(( f1_nd >= 1 )) || { echo "FAIL: GCC -fno-delete dropped f1's guard (got $f1_nd)" >&2; exit 1; }

echo "null-pointer check policy: PASS (9 shapes ratcheted at -O2/-O3, flag inert+silent, GCC sensitivity $def_total<$nd_total)"
