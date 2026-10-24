#!/usr/bin/env bash
# ============================================================================
# check_fp_select_minmax.sh — pin the one-instruction min/max lowering of FP
# selects over relational compares (`a > b ? a : b` and its seven siblings).
#
# THE OPTIMIZATION THIS PINS
# --------------------------
# A Select whose condition is a relational FP compare and whose arms are the
# compare's operands verbatim lowers to a single `maxsd`/`minsd` (or
# `maxss`/`minss`), replacing the whole ucomisd + cmov (SSE2) or vcmp +
# vblend (AVX) sequence. The hardware's SECOND-OPERAND-WINS rule on NaN and
# ties is exactly the C semantics of the ordered selects, with one subtlety
# that this gate exists to keep honest: the instruction follows the arms'
# ORIENTATION relative to the predicate —
#
#     ogt(a,b) ? a : b  ==  maxsd dst=a, src=b   (NaN → b, tie → b)
#     olt(a,b) ? a : b  ==  minsd dst=a, src=b
#     ogt(a,b) ? b : a  ==  minsd dst=b, src=a   (NaN → a, tie → a)
#     olt(a,b) ? b : a  ==  maxsd dst=b, src=a
#
# An earlier draft of the fold mapped BOTH `>`-forms to maxsd; the swapped
# shapes came back transposed in the differential hash below and the mapping
# was corrected. `ge`/`le` are deliberately NOT folded (their tie arm is the
# opposite of the hardware's tie arm) and must keep the compare path.
#
# THE GATE
# --------
#   1. BIT-EXACT DIFFERENTIAL — all eight shapes over a corner-value grid
#      (negatives, ±0.0, QNaN, SNaN; every ordered pair, both widths), hashed
#      by raw bit pattern, must equal the gcc-16 oracle's hash at
#      -O0/-O1/-O2/-O3/-Os. -O0/-O1 use the unfolded path, so the gate also
#      proves folded and unfolded agree — the strongest available statement
#      that the fold changed no bit.
#   2. SHAPE — the canonical scalar wrappers compile to exactly one
#      `maxsd`/`minsd`/`maxss` each at -O2 (the instruction the fold exists
#      to produce), and the deliberately-unfolded `ge`/`le` shapes contain
#      none (they must keep the compare path; folding them would flip ties).
# ============================================================================
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
ccc=${CCC:-$repo/target/fastbuild/lccc}
gcc=${GCC:-gcc-16}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

fail() { echo "check_fp_select_minmax: FAIL: $*" >&2; exit 1; }
[ -x "$ccc" ] || { echo "SKIP: $ccc not built" >&2; exit 0; }
"$gcc" --version 2>/dev/null | grep -q "16\.2" || { echo "SKIP: gcc-16.2 oracle not on PATH" >&2; exit 0; }

# --- 1. bit-exact differential over the corner grid -------------------------
cat >"$work/hash.c" <<'EOF'
#include <stdio.h>
#include <string.h>
typedef unsigned long long u64;
static u64 H; static void mix(u64 x){ H^=x; H*=1099511628211ULL; }
double s_gt(double a, double b)  { return a > b ? a : b; }
double s_lt(double a, double b)  { return a < b ? a : b; }
double s_gt_sw(double a, double b) { return a > b ? b : a; }
double s_lt_sw(double a, double b) { return a < b ? b : a; }
double s_ge(double a, double b)  { return a >= b ? a : b; }
double s_le(double a, double b)  { return a <= b ? a : b; }
float  f_gt(float a, float b)    { return a > b ? a : b; }
float  f_lt_sw(float a, float b) { return a < b ? b : a; }
#define DFN(name, fn) static void name(double a, double b){ u64 x; double r=fn(a,b); memcpy(&x,&r,8); mix(x); }
#define FFN(name, fn) static void name(float a, float b){ u64 x; unsigned y; float r=fn(a,b); memcpy(&y,&r,4); x=y; mix(x); }
DFN(h_gt, s_gt) DFN(h_lt, s_lt) DFN(h_gt_sw, s_gt_sw) DFN(h_lt_sw, s_lt_sw) DFN(h_ge, s_ge) DFN(h_le, s_le)
FFN(hf_gt, f_gt) FFN(hf_lt_sw, f_lt_sw)
int main(void){
    double dv[]={1.5,-2.25,0.0,-0.0,3.0,-3.0};
    double q,s; u64 qn=0x7ff8000000000000ULL,sn=0x7ff0000000000001ULL;
    memcpy(&q,&qn,8); memcpy(&s,&sn,8);
    double vals[8]; int n=0; for(int i=0;i<6;i++)vals[n++]=dv[i]; vals[n++]=q; vals[n++]=s;
    H=1469598103934665603ULL;
    for(int i=0;i<n;i++)for(int j=0;j<n;j++){
        h_gt(vals[i],vals[j]); h_lt(vals[i],vals[j]);
        h_gt_sw(vals[i],vals[j]); h_lt_sw(vals[i],vals[j]);
        h_ge(vals[i],vals[j]); h_le(vals[i],vals[j]);
    }
    printf("d=%016llx ",H);
    float fv[]={1.5f,-2.25f,0.0f,-0.0f}; float fq; unsigned fb=0x7fc00000u; memcpy(&fq,&fb,4);
    float fvals[5]; n=0; for(int i=0;i<4;i++)fvals[n++]=fv[i]; fvals[n++]=fq;
    H=1469598103934665603ULL;
    for(int i=0;i<n;i++)for(int j=0;j<n;j++){ hf_gt(fvals[i],fvals[j]); hf_lt_sw(fvals[i],fvals[j]); }
    printf("f=%016llx\n",H); return 0;
}
EOF
for opt in -O0 -O1 -O2 -O3 -Os; do
    "$gcc" "$opt" "$work/hash.c" -o "$work/g" || fail "gcc $opt build"
    g_out=$("$work/g")
    "$ccc" "$opt" "$work/hash.c" -o "$work/l" || fail "lccc $opt build"
    l_out=$("$work/l")
    [ "$g_out" = "$l_out" ] \
        || fail "$opt differential: lccc '$l_out' != gcc '$g_out' (NaN/tie/SNaN corners diverge)"
    echo "PASS: $opt bit-exact vs gcc-16 ($l_out)"
done

# --- 2. shape: one instruction for the foldable forms, none for ge/le -------
cat >"$work/shape.c" <<'EOF'
double dmax(double a, double b){ return a > b ? a : b; }
double dmin(double a, double b){ return a < b ? a : b; }
double dmax_sw(double a, double b){ return a > b ? b : a; }
float  fmax_(float a, float b){ return a > b ? a : b; }
double ge(double a, double b){ return a >= b ? a : b; }
double le(double a, double b){ return a <= b ? a : b; }
EOF
"$ccc" -O2 -S "$work/shape.c" -o "$work/shape.s" || fail "shape build"
for fn_insn in "dmax maxsd" "dmin minsd" "dmax_sw minsd" "fmax_ maxss"; do
    set -- $fn_insn
    fn=$1; insn=$2
    count=$(sed -n "/^${fn}:/,/^\.size\|^\.globl\|^ret/p" "$work/shape.s" \
        | grep -cE "^\s+${insn}\s")
    [ "$count" -eq 1 ] \
        || fail "$fn: expected exactly one $insn, found $count"
    echo "PASS: $fn -> single $insn"
done
for fn in ge le; do
    sed -n "/^${fn}:/,/^\.size\|^\.globl\|^ret/p" "$work/shape.s" \
        | grep -qE "^\s+maxsd\s|^\s+minsd\s" \
        && fail "$fn: ge/le must NOT fold to maxsd/minsd (tie semantics differ)" \
        || true
    echo "PASS: $fn correctly unfolded"
done

echo "check_fp_select_minmax: ok"
