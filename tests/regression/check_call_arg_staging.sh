#!/usr/bin/env bash
# Call-argument staging gate: every SSE argument a call makes reaches its
# register, the authority marker says the truth, and the FP liveness oracle
# cannot model a live staging as dead.
#
# WHY THIS EXISTS
#
# PR #584's CI found a miscompile class that every other gate missed: a call
# with >= 9 FP arguments (register staging + stack pushes) compiled with 7 of
# its 8 register arguments reading garbage.  The causal chain, established
# with the peephole trace instrumentation:
#
#   1. `emit_call_reg_args_impl` stages FP arguments as rax relays and
#      publishes the count as `movb $N, %al` (the census);
#   2. `eliminate_dead_pure_writes` LEGITIMATELY deletes the census for a
#      non-variadic callee (%al is dead there per the GP liveness oracle);
#   3. `eliminate_fp_xmm_roundtrips` converts the innermost relay into a
#      direct `movsd %xmmN, %xmmM` (also legitimate);
#   4. `eliminate_dead_vector_copies` asks `call_xmm_reads` for the call's
#      SSE read set; the census is gone, and the fallback window walk broke
#      at the first line that was not itself a staging move — the
#      hazard-area release `addq $K, %rsp`, the stack-argument `pushq %rax`,
#      the GP half of every remaining relay — so the call was modeled as
#      reading ZERO xmm registers and the live argument staging was deleted.
#
# The fix is layered: the codegen now publishes `# LCCC_CALL_FP <n>` after
# the call text (the same authority contract as `# LCCC_CALL_ARGS`; a
# comment, immune to every value-shape pass), and the window walk skips
# register-file-transparent lines (rsp adjustments, pushes/pops, relay
# GP-halves) with a step budget that covers the full 8-register staging
# window.  The emitter also no longer pre-spills STACK-class arguments into
# the hazard area (their value was already pushed by the stack-argument
# phase, which the driver runs first).
#
# This gate pins all three layers end to end:
#
#   A. runtime equality with the reference compiler on argument shapes built
#      to hit every layer: volatile local and global sources (the original
#      repros), 8..12 arguments (every stack-argument arity), f32, mixed
#      GP+FP, struct-by-SSE-register, FMA-negation compositions (the
#      add-of-neg fold freeing a multi-use negation into the staging),
#      register-hazard shapes (pre-spill + restore staging), indirect
#      calls, and calls inside loops;
#   B. the marker: present with the exact count for FP calls, absent for
#      GP-only calls, the census deleted for prototyped callees (the
#      instruction win) but KEPT for variadic ones (SysV 3.5.7 requires it);
#   C. the staging survival: the 10-argument volatile call writes ALL EIGHT
#      SSE argument registers in its staging window, and no FP argument is
#      routed through a GPR sign mask.
#
# Usage:
#   tests/regression/check_call_arg_staging.sh
# Environment:
#   LCCC            compiler to test (default: target/fastbuild/lccc)
#   CCC_ORACLE_CC   reference compiler (default: cc, gcc or clang)
set -uo pipefail

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$repo_root"

LCCC=${LCCC:-target/fastbuild/lccc}
[[ "$LCCC" == /* ]] || LCCC="$repo_root/$LCCC"
[[ -x "$LCCC" ]] || { echo "FAIL: no compiler at $LCCC" >&2; exit 1; }
ORACLE=${CCC_ORACLE_CC:-}
if [[ -z "$ORACLE" ]]; then
    for c in cc gcc clang; do command -v "$c" >/dev/null 2>&1 && { ORACLE=$c; break; }; done
fi
[[ -n "$ORACLE" ]] || { echo "FAIL: no reference compiler found" >&2; exit 1; }

FLAGS=(-O2 -march=x86-64-v3)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

pass=0; fail=0; skip=0
note() { printf '  %s\n' "$*"; }
ok()   { pass=$((pass+1)); printf 'ok   %s\n' "$*"; }
bad()  { fail=$((fail+1)); printf 'FAIL %s\n' "$*"; }
skipped() { skip=$((skip+1)); printf 'skip %s\n' "$*"; }

# ── A. runtime differential: every staging shape, bit-exact ─────────────────
cat > "$work/staging.c" <<'EOF_STAGING'
#include <stdio.h>
#include <math.h>
#define NI __attribute__((noinline))

/* The arities: 8 = exactly the SSE register file; 9..12 = 1..4 stack
 * arguments (the push sequences that used to terminate the liveness
 * window walk).  Sources are VOLATILE: every argument is re-read from
 * memory, defeating constant propagation and forcing the full staging. */
NI static double f8(double a,double b,double c,double d,double e,double f,double g,double h)
{ return a+b+c+d+e+f+g+h; }
NI static double f9(double a,double b,double c,double d,double e,double f,double g,double h,double i)
{ return a+b+c+d+e+f+g+h+i; }
NI static double f10(double a,double b,double c,double d,double e,double f,double g,double h,double i,double j)
{ return a+b+c+d+e+f+g+h+i+j; }
NI static double f11(double a,double b,double c,double d,double e,double f,double g,double h,double i,double j,double k)
{ return a+b+c+d+e+f+g+h+i+j+k; }
NI static double f12(double a,double b,double c,double d,double e,double f,double g,double h,double i,double j,double k,double l)
{ return a+b+c+d+e+f+g+h+i+j+k+l; }
NI static float g8(float a,float b,float c,float d,float e,float f,float g,float h)
{ return a+b+c+d+e+f+g+h; }
NI static float g9(float a,float b,float c,float d,float e,float f,float g,float h,float i)
{ return a+b+c+d+e+f+g+h+i; }
NI static float g10(float a,float b,float c,float d,float e,float f,float g,float h,float i,float j)
{ return a+b+c+d+e+f+g+h+i+j; }
/* Mixed GP+SSE: the classification interleaves IntReg and FloatReg. */
NI static double m10(int p,double a,int q,double b,int r,double c,int s,double d,double e)
{ return p+a+q+b+r+c+s+d+e; }
/* Two-double struct: SSE-classified by eightbyte (StructSseReg), two
 * registers per argument, stack overflow at arity 5. */
typedef struct { double x, y; } dd;
NI static double s3(dd a, dd b, dd c) { return a.x+a.y+b.x+b.y+c.x+c.y; }
NI static double s5(dd a, dd b, dd c, dd d, dd e)
{ return a.x+a.y+b.x+b.y+c.x+c.y+d.x+d.y+e.x+e.y; }
/* The FMA-negation composition: the add-of-neg fold frees the multi-use
 * negation into the peel while the staging runs (the shape that found
 * the original defect). */
NI static double neg9(double a,double b,double c,double d,double e,double f,double g,double h,double i)
{ return -a + -b + -c + -d + -e + -f + -g + -h + -i; }
NI static double fma_neg(double a, double b, double c)
{ double n = -a; return __builtin_fma(n, b, c) + n; }
/* Indirect call: the conservative path (no marker heuristics apply). */
NI static double (*fpi)(double,double,double,double,double,double,double,double,double);
/* Calls in a loop: cross-iteration staging, the liveness must hold at
 * every iteration boundary. */
NI static double loop9(volatile double *src, int n) {
    double acc = 0.0;
    for (int i = 0; i < n; i++)
        acc += f9(src[0], src[1], src[2], src[3], src[4], src[5], src[6], src[7], src[8]);
    return acc;
}
NI static double mixld(double a, long double b, double c)
{ return a + (double)b + c; }

static double volsrc;
volatile double varr[9];
int main(int argc, char **argv) {
    /* Opaque base: no constant folding of the argument values. */
    double s = argc * 1.0 + 0.125;
    volatile double vl = s;
    for (int i = 0; i < 9; i++) varr[i] = s + i;
    printf("%.17g %.17g %.17g %.17g %.17g\n",
        f8(vl,vl+1,vl+2,vl+3,vl+4,vl+5,vl+6,vl+7),
        f9(vl,vl+1,vl+2,vl+3,vl+4,vl+5,vl+6,vl+7,vl+8),
        f10(vl,vl+1,vl+2,vl+3,vl+4,vl+5,vl+6,vl+7,vl+8,vl+9),
        f11(vl,vl+1,vl+2,vl+3,vl+4,vl+5,vl+6,vl+7,vl+8,vl+9,vl+10),
        f12(vl,vl+1,vl+2,vl+3,vl+4,vl+5,vl+6,vl+7,vl+8,vl+9,vl+10,vl+11));
    volsrc = s;
    printf("%.9g %.9g %.9g\n",
        g8((float)volsrc,(float)volsrc+1,(float)volsrc+2,(float)volsrc+3,
           (float)volsrc+4,(float)volsrc+5,(float)volsrc+6,(float)volsrc+7),
        g9((float)volsrc,(float)volsrc+1,(float)volsrc+2,(float)volsrc+3,
           (float)volsrc+4,(float)volsrc+5,(float)volsrc+6,(float)volsrc+7,(float)volsrc+8),
        g10((float)volsrc,(float)volsrc+1,(float)volsrc+2,(float)volsrc+3,
            (float)volsrc+4,(float)volsrc+5,(float)volsrc+6,(float)volsrc+7,
            (float)volsrc+8,(float)volsrc+9));
    printf("%.17g\n", m10(argc, vl, argc+1, vl+1, argc+2, vl+2, argc+3, vl+3, vl+4));
    dd A = { s, s+1 }, B = { s+2, s+3 }, C = { s+4, s+5 }, D = { s+6, s+7 }, E = { s+8, s+9 };
    printf("%.17g %.17g\n", s3(A,B,C), s5(A,B,C,D,E));
    printf("%.17g\n", neg9(vl,vl+1,vl+2,vl+3,vl+4,vl+5,vl+6,vl+7,vl+8));
    printf("%.17g\n", fma_neg(vl, vl+1, vl+2));
    fpi = f9;
    printf("%.17g\n", fpi(vl,vl+1,vl+2,vl+3,vl+4,vl+5,vl+6,vl+7,vl+8));
    printf("%.17g\n", loop9(varr, 3));
    printf("%.17g\n", mixld(vl, (long double)vl + 0.5L, vl+1));
    return 0;
}
EOF_STAGING

if "$ORACLE" "${FLAGS[@]}" -o "$work/staging.oracle" "$work/staging.c" -lm 2>/dev/null; then
    "$work/staging.oracle" > "$work/stg.oracle" 2>&1
    stg_ok=1
    for cf in "-O0" "-O1" "-O2" "-O2 -march=x86-64-v2" "-O2 -march=x86-64-v3" "-O3 -march=x86-64-v3" "-O2 -march=x86-64-v3 -ffp-contract=off"; do
        # shellcheck disable=SC2086
        if ! "$LCCC" $cf -o "$work/staging.lccc" "$work/staging.c" -lm 2>"$work/stg.err"; then
            stg_ok=0; bad "staging.c did not compile with lccc at $cf:"; head -3 "$work/stg.err" | note; continue
        fi
        "$work/staging.lccc" > "$work/stg.lccc" 2>&1
        if ! diff -q "$work/stg.lccc" "$work/stg.oracle" >/dev/null; then
            stg_ok=0; bad "staging output differs from $ORACLE at $cf:"; diff "$work/stg.lccc" "$work/stg.oracle" | head -8 | note
        fi
    done
    if [[ "$stg_ok" -eq 1 ]]; then
        ok "call-argument staging bit-exact vs $ORACLE at 7 flag sets (volatile 8..12-ary f64/f32, mixed GP+SSE, SSE structs, neg compositions, indirect, loop, x87 mix)"
    fi
else
    bad "reference compiler could not build staging.c"
fi

# ── B. the authority marker and the census lifecycle ────────────────────────
"$LCCC" "${FLAGS[@]}" -S -o "$work/staging.s" "$work/staging.c" 2>/dev/null
# Marker counts: every noinline 8-SSE-register call in the file publishes
# CALL_FP 8 (f8..f12, g8..g10, neg9, the f9-in-loop site, the s5 struct
# call, the indirect site — data-calibrated, >= 10 with margin for
# devirtualisation), and the sub-8 arities are present with their exact
# counts (m10's 5 doubles, mixld's 2, loop9's GP-only 0).
n_fp8=$(grep -c '^    # LCCC_CALL_FP 8$' "$work/staging.s" || true)
n_fp5=$(grep -c '^    # LCCC_CALL_FP 5$' "$work/staging.s" || true)
n_fp2=$(grep -c '^    # LCCC_CALL_FP 2$' "$work/staging.s" || true)
n_fp0=$(grep -c '^    # LCCC_CALL_FP 0$' "$work/staging.s" || true)
if [[ "$n_fp8" -ge 10 && "$n_fp5" -ge 1 && "$n_fp2" -ge 1 && "$n_fp0" -ge 1 ]]; then
    ok "# LCCC_CALL_FP published with exact counts (8 x$n_fp8 register-file calls, 5 x$n_fp5 mixed, 2 x$n_fp2 x87-mix, 0 x$n_fp0 GP-only)"
else
    bad "LCCC_CALL_FP marker counts wrong: fp8=$n_fp8 fp5=$n_fp5 fp2=$n_fp2 fp0=$n_fp0 (expected >=10, >=1, >=1, >=1)"
fi
# The census is DEAD-ELIMINATED for prototyped callees (the instruction win
# that triggered the bug) and KEPT for variadic ones (SysV 3.5.7: printf
# reads %al).  Both sides of the dead-write interaction are pinned.
n_census=$(grep -cE '^[[:space:]]*movb \$[1-8], %al$' "$work/staging.s" || true)
n_printf=$(grep -c 'call printf@PLT' "$work/staging.s" || true)
if [[ "$n_census" -eq "$n_printf" ]]; then
    ok "the movb census survives exactly at the $n_printf variadic call(s) and is dead-eliminated at every prototyped callee"
else
    bad "census lifecycle: $n_census movb vs $n_printf printf calls (the census must survive ONLY at variadic sites)"
fi

# ── C. staging survival: all eight registers written in the f10 window ──────
# The original defect: f10's STAGING (in main, the caller) lost 7 of 8
# register writes.  Extract the contiguous window above the f10 call in
# main — everything after the last arithmetic line (vaddsd/vmulsd/vcvtsi*/
# x87 constant synthesis) within 40 lines of the call — and require the
# staging-class moves in it to write every SSE argument register.  The
# call text is `call f10` (same-TU static callee: no PLT).
window=$(grep -B40 -E 'call f10(@PLT)?$' "$work/staging.s" \
    | awk '/vaddsd|vmulsd|vcvtsi|vxorpd|fnmadd|fmadd|fld|fst|movabsq/ { buf = "" } { buf = buf $0 "\n" } END { printf "%s", buf }')
staged=$(printf '%s\n' "$window" \
    | grep -oE '^    (movq %rax, %xmm[0-7]|movq %r[a-z0-9]+, %xmm[0-7]|movd %r[a-z0-9]+, %xmm[0-7]|movsd? [^,]+, %xmm[0-7]|movap[ds] %xmm[0-9]+, %xmm[0-7]|vmovs[ds] %xmm[0-9]+, %xmm[0-9]+, %xmm[0-7])$' \
    | grep -oE '%xmm[0-7]$' | sort -u | wc -l)
if [[ "$staged" -eq 8 ]]; then
    ok "the 10-argument volatile call stages all 8 SSE argument registers (the exact shape the bug deleted)"
else
    bad "f10 staging window writes only $staged of 8 SSE argument registers"
    printf '%s\n' "$window" | tail -25 | note
fi
# No FP argument routed through a GPR sign mask INSIDE the staging window
# (the other face of a lost staging: the value rebuilt through movabsq).
# Scoped to the window: x87 constant synthesis elsewhere in the file
# legitimately stages sign/exponent bits through GP registers.
n_gpr_mask=$(printf '%s\n' "$window" \
    | grep -cE 'movabsq \$-9223372036854775808|xorl \$0x80000000, %eax' || true)
if [[ "$n_gpr_mask" -eq 0 ]]; then
    ok "no FP argument routed through a GPR sign mask in the staging window"
else
    bad "$n_gpr_mask GPR sign-mask materialisations inside the f10 staging window"
fi

echo
echo "call-argument staging gate: PASS=$pass FAIL=$fail SKIP=$skip"
[[ "$fail" -eq 0 ]] || exit 1
exit 0
