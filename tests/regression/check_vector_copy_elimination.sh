#!/usr/bin/env bash
# Vector copy elimination gate: the wins are real, the legality rules hold, and
# the numbers cannot silently regress.
#
# WHY THIS EXISTS
#
# The peephole's GP copy machinery declines every vector family by
# construction: `is_xmm_family` exists to keep families 24..39 away from the GP
# `reg_refs` bitmask, and `copy_propagation` (parses movq/movl),
# `coalesce_register_copies` (requires a literal "movq "), `relay_and_lea`
# (rejects anything that is not a `plain_gp_operand`) and `dead_writes` (GP
# pure-write prefixes) all bail out on a register they cannot represent. The
# codegen routes every scalar FP value through a fresh temporary, so the
# assembler text carried copy-in/copy-out brackets that nothing removed.
# `passes/vector_copy.rs` is the vector counterpart, and the archived oracle
# ranking at engineering/evidence/godbolt/s59-rank -- 51 benchmarks against gcc
# 16.2, clang 23.1.0, icc 2021.10.0 and icx, all at -O2 -march=x86-64-v3 -- is
# what sized the gap: `__builtin_floor` reached the assembler as three
# instructions where ICX emits one `vroundsd`, `double t = a; t += b; return t;`
# as four where GCC, Clang and ICX all emit one `vaddsd`, and `__builtin_fma` as
# seven where all three emit one fused instruction.
#
# A peephole that deletes register copies is one wrong liveness answer away from
# a miscompile, and this tree has been bitten by exactly that: the
# induction-variable widening that scripts/check_benchmark_outputs.sh exists to
# catch passed 563 regression tests and silently changed SQLite's output. So
# this gate asserts three things, in increasing order of how much they matter:
#
#   A. the instruction shapes the oracles establish -- the wins are real, and
#      the one residual copy in the set is named rather than hidden;
#   B. runtime equality with GCC on kernels built to hit every legality rule: a
#      live-in temporary, a packed read of a scalar copy, a call inside a
#      bracket, a branch inside a bracket, a two-register FP return, ymm
#      aliasing an xmm, float and double in one function, dead and live results
#      side by side, all four FMA families, and (where the host has it) AVX-512;
#   C. a corpus instruction ratchet, so a change that reintroduces the copies
#      fails here with the benchmark named instead of shipping as a slower
#      compiler nobody measured.
#
# Usage:
#   tests/regression/check_vector_copy_elimination.sh
# Environment:
#   LCCC            compiler to test (default: target/fastbuild/lccc)
#   CCC_ORACLE_CC   reference compiler for part B (default: cc, gcc or clang)
set -uo pipefail

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$repo_root"

# Absolute before anything runs: a relative LCCC= reaches a subshell that
# changed directory as a path that does not exist, and the resulting 127 reads
# as a compiler failure rather than a missing file.
LCCC=${LCCC:-target/fastbuild/lccc}
[[ "$LCCC" == /* ]] || LCCC="$repo_root/$LCCC"
[[ -x "$LCCC" ]] || { echo "FAIL: no compiler at $LCCC" >&2; exit 1; }
ORACLE=${CCC_ORACLE_CC:-}
if [[ -z "$ORACLE" ]]; then
    for c in cc gcc clang; do command -v "$c" >/dev/null 2>&1 && { ORACLE=$c; break; }; done
fi

FLAGS=(-O2 -march=x86-64-v3)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

pass=0; fail=0; skip=0
note() { printf '  %s\n' "$*"; }
ok()   { pass=$((pass+1)); printf 'ok   %s\n' "$*"; }
bad()  { fail=$((fail+1)); printf 'FAIL %s\n' "$*"; }
skipped() { skip=$((skip+1)); printf 'skip %s\n' "$*"; }

# The instruction lines of one function body, `ret` included, with labels,
# directives, comments and blanks dropped.  [[:space:]] and not \s: GCC indents
# with tabs and POSIX awk and grep have no \s, so a \s pattern silently counts
# .cfi_startproc as an instruction and every comparison against an oracle comes
# out one too high.
fnbody() {
    awk -v f="$2:" '$0 == f { p = 1; next } p && /^[[:space:]]*ret/ { print "ret"; exit } p' "$1" \
        | grep -vE '^[[:space:]]*(\.|#|$)' | sed 's/^[[:space:]]*//'
}
insns() { grep -vE '^[[:space:]]*(\.|#|$)' "$1" | grep -vE ':[[:space:]]*$' | sed 's/^[[:space:]]*//'; }

# A vector register-to-register copy: the pattern this pass exists to remove.
# A memory operand on either side is not a copy -- `movsd .LC0(%rip), %xmm0` is
# a constant load, and deleting it would delete a value.
COPY_RE='^v?mov(sd|ss|apd|aps|upd|ups|dqa|dqu|dq|q)[[:space:]]+%[xyz]mm[0-9]+,[[:space:]]*(%[xyz]mm[0-9]+,[[:space:]]*)?%[xyz]mm[0-9]+$'

# ── A. the shapes the oracles establish ───────────────────────────────────────
cat > "$work/shapes.c" <<'EOF'
double floor_d(double x) { return __builtin_floor(x); }
double ceil_d(double x)  { return __builtin_ceil(x); }
double trunc_d(double x) { return __builtin_trunc(x); }
double rint_d(double x)  { return __builtin_rint(x); }
float  floor_f(float x)  { return __builtin_floorf(x); }
double add2(double a, double b) { double t = a; t += b; return t; }
double mul2(double a, double b) { double t = a; t *= b; return t; }
double fma3(double a, double b, double c) { return __builtin_fma(a, b, c); }
double csign(double x, double y) { return __builtin_copysign(x, y); }
double chain(double a) { double t = a; t = t + 1.0; t = t * 2.0; t = t - 3.0; return t; }
double neg_d(double x) { return -x; }
float  neg_f(float x)  { return -x; }
double mul_add(double a, double b, double c) { return a * b + c; }
double sq_add(double x, double c) { return x * x + c; }
double fma_mem(double a, const double *p, double c) { return __builtin_fma(a, *p, c); }
EOF
"$LCCC" "${FLAGS[@]}" -ffp-contract=fast -S -o "$work/shapes.s" "$work/shapes.c" \
    || { echo "FAIL: shapes.c did not compile" >&2; exit 1; }

# Each of these is one instruction plus the ret, which is what GCC 16.2, Clang
# 23.1 and ICX all emit for the same source.  The immediate is the ABI-visible
# part of a directed round (9 floor, 10 ceil, 11 trunc, 4 rint) and is asserted
# literally: a pass that folded the copies but corrupted the rounding mode would
# still be "one instruction".
expect_shape() { # name, exact instruction
    local body n
    body=$(fnbody "$work/shapes.s" "$1")
    n=$(printf '%s\n' "$body" | grep -c . || true)
    if [[ "$n" -eq 2 ]] && printf '%s\n' "$body" | head -1 | grep -qxF "$2"; then
        ok "$1 is exactly \`$2' plus ret (gcc/clang/icx parity)"
    else
        bad "$1 should be \`$2' plus ret, got $n lines:"; printf '%s\n' "$body" | note
    fi
}
expect_shape floor_d 'vroundsd $9, %xmm0, %xmm0, %xmm0'
expect_shape ceil_d  'vroundsd $10, %xmm0, %xmm0, %xmm0'
expect_shape trunc_d 'vroundsd $11, %xmm0, %xmm0, %xmm0'
expect_shape rint_d  'vroundsd $4, %xmm0, %xmm0, %xmm0'
expect_shape floor_f 'vroundss $9, %xmm0, %xmm0, %xmm0'
expect_shape add2    'vaddsd %xmm1, %xmm0, %xmm0'
expect_shape mul2    'vmulsd %xmm1, %xmm0, %xmm0'
# The FMA is the case that needed a rotation rather than a deletion: every
# VFMADD form reads its own destination as the accumulator, so the copy-out
# cannot be served by retargeting.  231 becomes 213, whose destination is a
# multiplicand and whose src2 is the addend -- the instruction Clang and ICX
# emit, and one of the two GCC picks (it takes 132).
expect_shape fma3    'vfmadd213sd %xmm2, %xmm1, %xmm0'
# The contracted `a * b + c` arrives in the 132 form with a two-copy swap in
# front of it and a copy-out behind; re-encoding into the return register is
# GCC's exact text.  `x * x + c` is the same rule with one value in two roles.
# A memory multiplicand rides operand 0 in whichever form allows it.
expect_shape mul_add 'vfmadd132sd %xmm1, %xmm2, %xmm0'
expect_shape sq_add  'vfmadd213sd %xmm1, %xmm0, %xmm0'
expect_shape fma_mem 'vfmadd132sd (%rdi), %xmm1, %xmm0'
# Negation is one sign-bit xor against a rodata mask, computed straight into
# the return register -- GCC, Clang and ICX all agree.  It used to be a
# five-instruction GPR round trip (movq / movabsq / xorq / movq / movsd).  The
# pool label is whatever the constant pool assigned; the mask itself is checked
# by the runtime differential below.
expect_shape_re() { # name, regex for the one instruction
    local body n
    body=$(fnbody "$work/shapes.s" "$1")
    n=$(printf '%s\n' "$body" | grep -c . || true)
    if [[ "$n" -eq 2 ]] && printf '%s\n' "$body" | head -1 | grep -qE "$2"; then
        ok "$1 is exactly one \`$(printf '%s\n' "$body" | head -1 | cut -d' ' -f1)' plus ret (gcc/clang/icx parity)"
    else
        bad "$1 should match /$2/ plus ret, got $n lines:"; printf '%s\n' "$body" | note
    fi
}
expect_shape_re neg_d '^vxorpd \.LCFP_[0-9]+\(%rip\), %xmm0, %xmm0$'
expect_shape_re neg_f '^vxorps \.LCFP_[0-9]+\(%rip\), %xmm0, %xmm0$'

# copysign is three bit operations, which is Clang's answer and one better than
# GCC's four.  The two copies that used to sit in front of the bit ops are what
# widening plus propagation removed: a 64-bit copy cannot serve a 128-bit
# vandpd, and the temporary is private, so the bits the copy did not define were
# nobody's and may become the source's.
cs=$(fnbody "$work/shapes.s" csign)
if [[ "$(printf '%s\n' "$cs" | grep -c .)" -eq 4 ]] \
   && ! printf '%s\n' "$cs" | grep -qE "$COPY_RE" \
   && printf '%s\n' "$cs" | grep -q 'vorpd'; then
    ok "copysign is three bit ops plus ret (clang parity, one better than gcc)"
else
    bad "copysign should be andpd/andpd/orpd plus ret with no register copy:"
    printf '%s\n' "$cs" | note
fi

# No vector register copy survives anywhere in this file.  `chain` used to end
# in `vfmsub132sd ..., %xmm2` / `movsd %xmm2, %xmm0` because the codegen put
# the constant 3.0 in %xmm0; the FMA re-encoding now writes %xmm0 directly and
# `chain` is three instructions, GCC's count.
copies=$(insns "$work/shapes.s" | grep -cE "$COPY_RE" || true)
if [[ "$copies" -eq 0 ]]; then
    ok "no vector register copy survives the whole shape set"
else
    bad "$copies vector register copies survived, budget is 0:"
    insns "$work/shapes.s" | grep -E "$COPY_RE" | note
fi
ch=$(fnbody "$work/shapes.s" chain | grep -c . || true)
if [[ "$ch" -eq 4 ]]; then
    ok "chain is three instructions plus ret (gcc parity)"
else
    bad "chain should be three instructions plus ret, got $ch lines:"
    fnbody "$work/shapes.s" chain | note
fi

# ── B. runtime equality with GCC over kernels built for the legality rules ────
if [[ -z "$ORACLE" ]]; then
    skipped "no reference compiler for the runtime differential (set CCC_ORACLE_CC)"
else
    cat > "$work/runtime.c" <<'EOF'
    /* Every function here is shaped to hit one legality rule of the vector copy
     * passes.  They print with %.17g so a single bit of difference is visible:
     * these are the values a wrong coalesce, a wrong widening, a wrong FMA
     * rotation or a missed kill would change. */
    #include <stdio.h>
    #include <math.h>

    /* Rule 3/5: a packed read of a register that a scalar copy defined.  The
     * vector type makes the second lane observable, so a widening that changed
     * the upper bits from the caller's leftovers to something else shows up. */
    typedef double v2df __attribute__((vector_size(16)));
    static double packed_read_of_scalar_copy(double a, double b) {
        v2df v = { a, b };
        v += (v2df){ __builtin_floor(a), __builtin_ceil(b) };
        return v[0] + v[1];
    }

    /* Rule 6: a call inside what looks like a bracket.  Every vector register
     * is caller-saved under SysV, so coalescing across this call loses the
     * value, and the call also reads %xmm0-%xmm7 as arguments. */
    __attribute__((noinline)) static double opaque(double x) { return x * 1.5; }
    static double call_inside_bracket(double a, double b) {
        double t = a;
        t = opaque(t);
        t += b;
        return t;
    }

    /* Rule 6 again, with a branch: only one half of the bracket runs, so a
     * transformation that assumed both did would compute the wrong value. */
    static double branch_inside_bracket(double a, int sel) {
        double t = a;
        if (sel) t = __builtin_floor(t); else t = __builtin_ceil(t);
        return t + a;
    }

    /* Rule 4: the temporary is live outside the bracket, so it is not private
     * and must not be renamed away. */
    static double temporary_live_outside(double a) {
        double t = a;
        double u = __builtin_trunc(t);
        t = __builtin_floor(t);
        return t + u + a;
    }

    /* The FP return registers: a two-double struct comes home in %xmm0 and
     * %xmm1, and `ret` reads both implicitly -- a reader no textual scan of the
     * instruction's operands can see.  (A _Complex return would test the same
     * thing but needs <complex.h> for _Complex_I, and the struct form has no
     * libc dependency to disagree about.) */
    typedef struct { double lo, hi; } dpair;
    static dpair two_register_return(double a, double b) {
        dpair r;
        r.lo = __builtin_fma(a, b, 1.0);
        r.hi = __builtin_copysign(a, b) + __builtin_rint(b);
        return r;
    }

    /* The FMA accumulator bracket, forced: noinline and a direct return, so the
     * value has to travel through a temporary and back into %xmm0.  Without
     * these the rotation this gate exists to protect never appears in the
     * binary, and a bit-exact run would prove nothing about it -- the assertion
     * below is what makes that impossible to miss.  The four families come from
     * the four sign combinations; which encoding each lands in is the backend's
     * choice, and every one must survive bit-exactly. */
    __attribute__((noinline)) static double fma_add(double a, double b, double c) {
        return __builtin_fma(a, b, c);
    }
    __attribute__((noinline)) static double fma_sub(double a, double b, double c) {
        return a * b - c;
    }
    __attribute__((noinline)) static double fma_nadd(double a, double b, double c) {
        return c - a * b;
    }
    __attribute__((noinline)) static double fma_nsub(double a, double b, double c) {
        return -a * b - c;
    }
    __attribute__((noinline)) static float fmaf_add(float a, float b, float c) {
        return __builtin_fmaf(a, b, c);
    }
    __attribute__((noinline)) static double fma_chain(double a, double b, double c) {
        double t = __builtin_fma(a, b, c);
        return __builtin_fma(t, b, a) - __builtin_fma(c, c, t);
    }

    /* The rotation on values whose product and addend are the same register:
     * the rotated form reads its destination as a source, so this is the shape
     * where a wrong rotation is visible rather than merely different. */
    static double fma_forms(double a, double b, double c) {
        double r = __builtin_fma(a, b, c);
        r += __builtin_fma(-a, b, -c);
        r += __builtin_fma(a, -b, c);
        r -= __builtin_fma(-a, -b, c);
        r += __builtin_fmaf((float)a, (float)b, (float)c);
        double t = a;
        t = __builtin_fma(t, b, t);
        t = __builtin_fma(t, t, c);
        return r + t;
    }

    /* Scalar and vector traffic on one physical register: %ymm2 aliases %xmm2,
     * so a pass that tracks only one spelling gets this wrong. */
    static double ymm_and_xmm_aliasing(double a) {
        v2df v = { a, a + 1.0 };
        double s = __builtin_floor(a);
        v *= (v2df){ s, s };
        return v[0] - v[1];
    }

    /* A loop whose accumulator the vectorizer puts in a register that also
     * serves as a copy destination on some iterations. */
    static double reduction_over_array(const double *xs, int n) {
        double acc = 0.0;
        for (int i = 0; i < n; i++) acc += __builtin_fabs(xs[i]) + __builtin_floor(xs[i]);
        return acc;
    }

    /* float and double in one function: 32- and 64-bit copies of the same
     * register family, which a width-blind pass conflates. */
    static double mixed_widths(float f, double d) {
        float tf = f; tf = __builtin_floorf(tf); tf *= 2.0f;
        double td = d; td = __builtin_ceil(td);  td *= 3.0;
        return (double)tf + td;
    }

    /* Dead and live results side by side: the unused one must not keep its
     * register alive, and the used one must survive. */
    static double dead_and_live(double a, double b) {
        double unused = __builtin_floor(a) + __builtin_ceil(b);
        (void)unused;
        double used = __builtin_trunc(a) - __builtin_rint(b);
        return used;
    }

    int main(void) {
        static const double xs[] = {
            -3.7, -2.5, -1.0, -0.5, -0.0, 0.0, 0.5, 1.0, 2.5, 3.7, 1e18, -1e18,
            1e-300, 4503599627370497.0 /* 2^52+1: rounding is observable */
        };
        const unsigned n = sizeof xs / sizeof *xs;
        double r = 0.0;
        for (unsigned i = 0; i < n; i++) {
            double a = xs[i], b = xs[(i + 3) % n], c = xs[(i + 7) % n];
            r += packed_read_of_scalar_copy(a, b);
            r += call_inside_bracket(a, b);
            r += branch_inside_bracket(a, (int)i & 1);
            r += temporary_live_outside(a);
            r += fma_forms(a, b, c);
            r += ymm_and_xmm_aliasing(a);
            r += reduction_over_array(xs, (int)n);
            r += mixed_widths((float)a, b);
            r += dead_and_live(a, b);
            r += fma_add(a, b, c) + fma_sub(a, b, c);
            r += fma_nadd(a, b, c) + fma_nsub(a, b, c);
            r += fmaf_add((float)a, (float)b, (float)c);
            r += fma_chain(a, b, c);
            dpair p = two_register_return(a, b);
            r += p.lo + p.hi;
        }
        printf("%.17g\n", r);
        printf("%.17g %.17g %.17g\n",
               packed_read_of_scalar_copy(-2.5, 3.7),
               ymm_and_xmm_aliasing(-0.5),
               fma_forms(-0.0, 3.7, -0.0));
        dpair p = two_register_return(2.5, -3.7);
        printf("%.17g %.17g\n", p.lo, p.hi);
        return 0;
    }
EOF
    "$LCCC" "${FLAGS[@]}" -o "$work/runtime.lccc" "$work/runtime.c" 2>"$work/lccc.err"
    lccc_rc=$?
    "$ORACLE" -O2 -march=x86-64-v3 -o "$work/runtime.oracle" "$work/runtime.c" 2>/dev/null
    oracle_rc=$?
    if [[ $lccc_rc -ne 0 ]]; then
        bad "runtime.c did not compile with lccc:"; head -5 "$work/lccc.err" | note
    elif [[ $oracle_rc -ne 0 ]]; then
        skipped "reference compiler could not build runtime.c"
    else
        "$work/runtime.lccc" > "$work/out.lccc" 2>&1; rc_l=$?
        "$work/runtime.oracle" > "$work/out.oracle" 2>&1; rc_o=$?
        if [[ $rc_l -ne $rc_o ]]; then
            bad "exit status differs: lccc=$rc_l $ORACLE=$rc_o"
        elif ! diff -q "$work/out.lccc" "$work/out.oracle" >/dev/null; then
            bad "runtime output differs from $ORACLE on the legality-rule kernels:"
            diff "$work/out.lccc" "$work/out.oracle" | head -10 | note
        else
            ok "runtime bit-exact vs $ORACLE on $(grep -c . "$work/out.lccc") output lines (every legality rule exercised)"
        fi
        # Anti-vacuity: the rotation must actually have happened in this
        # binary's output, or the bit-exactness above proves nothing about it.
        # A hard failure, not a note -- a gate that quietly stops exercising the
        # code it guards is worse than no gate.
        "$LCCC" "${FLAGS[@]}" -S -o "$work/runtime.s" "$work/runtime.c" 2>/dev/null
        n_fma=$(grep -cE '^[[:space:]]*vf(n?m(add|sub))(132|213|231)(sd|ss)' "$work/runtime.s" || true)
        n_213=$(grep -cE '^[[:space:]]*vf(n?m(add|sub))213(sd|ss)' "$work/runtime.s" || true)
        n_copies=$(insns "$work/runtime.s" | grep -cE "$COPY_RE" || true)
        if [[ "$n_213" -gt 0 ]]; then
            ok "the differential really exercises the rotation: $n_fma FMA sites, $n_213 in the rotated 213 form"
        else
            bad "no 213-rotated FMA in the runtime kernel ($n_fma FMA sites): the differential would not exercise the rotation"
        fi
        # And the copies this pass exists to remove must be largely gone from a
        # kernel this copy-heavy.  48 is the measured residual, inventoried by
        # shape in FOLLOWUP-2026-09-19E: 14 are VEX merge-form copies whose
        # zeroed upper bits a 128-bit read genuinely needs (refusing them is
        # correct), and the rest are the FMA form-selection cases that belong to
        # instruction selection.  The ratchet is here so neither grows silently.
        if [[ "$n_copies" -le 42 ]]; then
            ok "runtime kernel carries $n_copies vector register copies (budget 42, inventoried in FOLLOWUP-2026-09-19E)"
        else
            bad "runtime kernel carries $n_copies vector register copies, budget is 42"
            insns "$work/runtime.s" | grep -E "$COPY_RE" | head -10 | note
        fi
    fi

    # ── D. FMA form algebra, end to end ─────────────────────────────────────
    # The re-encoding rules (132/213/231, memory role at operand 0, result in a
    # role register) are proven exhaustively in `fma_forms::tests`; this kernel
    # checks the compiled program.  Two kinds of kernel, because they need two
    # kinds of proof:
    #
    #  * `__builtin_fma(±a, ±b, ±c)` has ONE meaning -- a single rounding -- so
    #    its output is compared bit-exactly against the reference compiler,
    #    whatever instructions either side chose.
    #  * `a * b + c` under -ffp-contract=fast may legally be fused or not, so
    #    it is not comparable across compilers; instead each contracted kernel
    #    is compared, inside the same binary, against its builtin twin.  Equal
    #    bits prove the contracted path computed the fused value with the
    #    roles in the right places; a swapped role or a stray copy shows up as
    #    a different number.
    #
    # Operands are chosen so a fused result differs from an unfused one (the
    # products need 106 bits), sign families differ, and both element widths
    # and every memory placement appear.  Every kernel is noinline with its
    # own argument order, so the allocator's choice of home differs across
    # them and every encoding gets its turn.
    cat > "$work/fma_algebra.c" <<'EOF_ALGEBRA'
    #include <stdio.h>
    #include <math.h>
    #include <string.h>
    #include <stdint.h>
    #define NI __attribute__((noinline))
    /* ── builtins: pinned semantics, compared against the oracle ── */
    NI static double b_madd(double a, double b, double c) { return __builtin_fma(a, b, c); }
    NI static double b_msub(double a, double b, double c) { return __builtin_fma(a, b, -c); }
    NI static double b_nmadd(double a, double b, double c) { return __builtin_fma(-a, b, c); }
    NI static double b_nmsub(double a, double b, double c) { return __builtin_fma(-a, b, -c); }
    /* ── builtin operand-negation shapes the IR peel absorbs ──
     * (fma_neg_peel.rs: the Neg instructions feeding the intrinsic fold
     * into the family selection, so each of these is ONE instruction —
     * b_cancel and b_chainneg land on the PLAIN family because their two
     * product-side negations cancel, b_named's negations are non-adjacent
     * named locals, b_nmsubf is the F32 width.) */
    NI static double b_cancel(double a, double b, double c) { return __builtin_fma(-a, -b, c); }
    NI static double b_named(double a, double b, double c) { double na = -a, nc = -c; return __builtin_fma(na, b, nc); }
    NI static double b_chainneg(double a, double b, double c) { return __builtin_fma(-(-a), b, c); }
    NI static float b_nmsubf(float a, float b, float c) { return __builtin_fmaf(-a, b, -c); }
    /* negative control: the negation has a SECOND consumer (the return
     * reads n again), so the single-use discipline must keep it
     * materialised — exactly one vxorpd in the whole file below. */
    NI static double b_shared_neg(double a, double b, double c) { double n = -a; return __builtin_fma(n, b, c) + n; }
    NI static double b_madd_p(double a, double b, const double *p) { return __builtin_fma(a, b, *p); }
    NI static double b_madd_pm(double a, const double *p, double c) { return __builtin_fma(*p, a, c); }
    NI static float b_maddf(float a, float b, float c) { return __builtin_fmaf(a, b, c); }
    NI static float b_maddf_pm(float a, const float *p, float c) { return __builtin_fmaf(a, *p, c); }
    NI static double b_chain(double a, double b, double c, double d) {
        return __builtin_fma(a, b, __builtin_fma(c, d, 1.0));
    }
    NI static double b_reuse(double a, double b, double c) { return __builtin_fma(a, b, c) + c; }
    NI static double b_dot(const double *x, const double *y, int n) {
        double s = 0.0;
        for (int i = 0; i < n; i++) s = __builtin_fma(x[i], y[i], s);
        return s;
    }
    /* ── contracted: compared against the builtin twin in this binary ── */
    /* result in a, a is a multiplicand (132/213 territory) */
    NI static double madd_a(double a, double b, double c) { return a * b + c; }
    NI static double msub_a(double a, double b, double c) { return a * b - c; }
    NI static double nmadd_a(double a, double b, double c) { return c - a * b; }
    /* result in a, a is the addend (231 territory) */
    NI static double madd_c(double a, double b, double c) { return b * c + a; }
    NI static double msub_c(double a, double b, double c) { return b * c - a; }
    NI static double nmadd_c(double a, double b, double c) { return a - b * c; }
    /* a multiplicand from memory, in both operand positions; the addend from memory */
    NI static double madd_m1(double a, const double *p, double c) { return a * *p + c; }
    NI static double madd_m2(double a, const double *p, double c) { return *p * a + c; }
    NI static double madd_ma(double a, double b, const double *p) { return a * b + *p; }
    /* one value in two roles */
    NI static double sq_add(double x, double c) { return x * x + c; }
    NI static double self_add(double x, double c) { return x * c + x; }
    NI static double self_add2(double x, double c) { return c * x + x; }
    /* negation shapes: the four sign families plus the two double
     * negations that fold back to plain fmadd.  Each is ONE instruction
     * under -ffp-contract=fast -- GCC contracts all six this way (measured
     * on the local oracle; the vfnmsub half was unreachable in lccc before
     * the signed-FMA hook: the Neg survived as a separate vxorpd). */
    NI static double neg_prod_sub(double a, double b, double c) { return -(a * b) - c; }
    NI static double neg_prod_add(double a, double b, double c) { return -(a * b) + c; }
    NI static double neg_sum(double a, double b, double c) { return -(a * b + c); }
    NI static double neg_diff(double a, double b, double c) { return -(a * b - c); }
    NI static double sub_neg_prod(double a, double b, double c) { return c - -(a * b); }
    NI static double neg_neg_sub(double a, double b, double c) { return -(-(a * b) - c); }
    /* float */
    NI static float maddf_a(float a, float b, float c) { return a * b + c; }
    NI static float maddf_m(float a, const float *p, float c) { return *p * a + c; }
    NI static float nmaddf_c(float a, float b, float c) { return a - b * c; }
    NI static float neg_prod_subf(float a, float b, float c) { return -(a * b) - c; }
    NI static float neg_sumf(float a, float b, float c) { return -(a * b + c); }
    static int mismatches = 0;
    /* Exact-bit comparison with one documented freedom: a NaN RESULT may
     * carry any payload and sign (implementation-defined in IEEE 754;
     * hardware fma and libm's software fma exercise the choice
     * differently, and operand position in the instruction decides which
     * input's payload propagates).  Two NaNs therefore compare equal.
     * Everything else -- including the SIGN OF ZERO, which a role-swapped
     * or wrongly-negated family flips -- must match bit for bit. */
    static void same(const char *name, double got, double want) {
        uint64_t g, w;
        memcpy(&g, &got, sizeof g);
        memcpy(&w, &want, sizeof w);
        if (g != w && !(got != got && want != want)) {
            mismatches++;
            printf("MISMATCH %s %.17g != %.17g\n", name, got, want);
        }
    }
    /* Print helper for the oracle-compared lines: finite numbers,
     * infinities and signed zeros print exactly (their bits are
     * IEEE-pinned); NaN canonicalizes to one spelling so payload and sign
     * differences -- the implementation-defined freedom above -- do not
     * read as disagreements between two correct compilers. */
    static const char *canon(char *buf, double v) {
        if (v != v) return "nan";
        sprintf(buf, "%.17g", v);
        return buf;
    }
    int main(void) {
        /* The finite core keeps products that need 106 bits (a fused and a
         * split evaluation differ); the boundary tail (zeros, infinities,
         * overflow, subnormals, both NaN signs) exercises the paths where
         * FMA semantics are edge-defined rather than merely rounded. */
        static const double xs[] = {
            1.0000000000000002, -3.0000000000000004, 0.30000000000000004, -7.000000000000001,
            1e16, -1e-16, 4503599627370497.0, -0.1, 2.5, -1234567.891,
            0.0, -0.0, INFINITY, -INFINITY, 1e308, 1e-308, NAN, -NAN
        };
        static const float fs[] = { 1.00000012f, -3.0000002f, 0.3f, -7.0000005f, 1e8f, -1e-8f,
                                    16777217.0f, -0.1f, 2.5f, -1234.5678f,
                                    0.0f, -0.0f, INFINITY, -INFINITY, 1e30f, 1e-30f, NAN, -NAN };
        const int n = (int)(sizeof xs / sizeof *xs);
        double acc = 0.0;
        for (int i = 0; i < n; i++) {
            double a = xs[i], b = xs[(i + 3) % n], c = xs[(i + 7) % n], d = xs[(i + 1) % n];
            float fa = fs[i], fb = fs[(i + 3) % n], fc = fs[(i + 7) % n];
            char b1[40], b2[40], b3[40], b4[40], b5[40], b6[40], b7[40], b8[40], b9[40], b10[40];
            /* oracle-compared (NaN-spelling canonicalized) */
            printf("%d %s %s %s %s %s %s %s %s %s %s\n", i,
                   canon(b1, b_madd(a, b, c)), canon(b2, b_msub(a, b, c)),
                   canon(b3, b_nmadd(a, b, c)), canon(b4, b_nmsub(a, b, c)),
                   canon(b5, b_madd_p(a, b, &c)), canon(b6, b_madd_pm(a, &b, c)),
                   canon(b7, (double)b_maddf(fa, fb, fc)), canon(b8, (double)b_maddf_pm(fa, &fb, fc)),
                   canon(b9, b_chain(a, b, c, d)), canon(b10, b_reuse(a, b, c)));
            /* self-compared: the contracted kernel must equal its builtin */
            same("madd_a", madd_a(a, b, c), b_madd(a, b, c));
            same("msub_a", msub_a(a, b, c), b_msub(a, b, c));
            same("nmadd_a", nmadd_a(a, b, c), b_nmadd(a, b, c));
            same("madd_c", madd_c(a, b, c), b_madd(b, c, a));
            same("msub_c", msub_c(a, b, c), b_msub(b, c, a));
            same("nmadd_c", nmadd_c(a, b, c), b_nmadd(b, c, a));
            same("madd_m1", madd_m1(a, &b, c), b_madd(a, b, c));
            same("madd_m2", madd_m2(a, &b, c), b_madd(b, a, c));
            same("madd_ma", madd_ma(a, b, &c), b_madd(a, b, c));
            same("sq_add", sq_add(a, c), b_madd(a, a, c));
            same("self_add", self_add(a, c), b_madd(a, c, a));
            same("self_add2", self_add2(a, c), b_madd(c, a, a));
            same("maddf_a", (double)maddf_a(fa, fb, fc), (double)b_maddf(fa, fb, fc));
            same("maddf_m", (double)maddf_m(fa, &fb, fc), (double)b_maddf(fb, fa, fc));
            same("nmaddf_c", (double)nmaddf_c(fa, fb, fc), (double)__builtin_fmaf(-fb, fc, fa));
            /* the sign algebra: each source shape equals its signed
             * builtin twin, including the two double negations that land
             * on the plain family */
            same("neg_prod_sub", neg_prod_sub(a, b, c), b_nmsub(a, b, c));
            same("neg_prod_add", neg_prod_add(a, b, c), b_nmadd(a, b, c));
            same("neg_sum", neg_sum(a, b, c), -b_madd(a, b, c));
            same("neg_diff", neg_diff(a, b, c), -b_msub(a, b, c));
            same("sub_neg_prod", sub_neg_prod(a, b, c), b_madd(a, b, c));
            same("neg_neg_sub", neg_neg_sub(a, b, c), b_madd(a, b, c));
            same("neg_prod_subf", (double)neg_prod_subf(fa, fb, fc), (double)__builtin_fmaf(-fa, fb, -fc));
            same("neg_sumf", (double)neg_sumf(fa, fb, fc), (double)-__builtin_fmaf(fa, fb, fc));
            /* the operand-negation peel: each builtin sign variant equals
             * its spelled-out twin, the two cancellations equal the plain
             * family, and the multi-use control keeps its arithmetic while
             * materialising the shared negation. */
            same("b_cancel", b_cancel(a, b, c), b_madd(a, b, c));
            same("b_named", b_named(a, b, c), b_nmsub(a, b, c));
            same("b_chainneg", b_chainneg(a, b, c), b_madd(a, b, c));
            same("b_nmsubf", (double)b_nmsubf(fa, fb, fc), (double)__builtin_fmaf(-fa, fb, -fc));
            same("b_shared_neg", b_shared_neg(a, b, c), b_nmadd(a, b, c) - a);
            acc += madd_a(a, b, c) + madd_c(a, b, c) + sq_add(a, c);
        }
        char b1[40];
        printf("%s\n", canon(b1, b_dot(xs, xs + 1, n - 1)));
        printf("contracted kernels: %d mismatches\n", mismatches);
        (void)acc;
        return mismatches != 0;
    }
EOF_ALGEBRA
    "$LCCC" "${FLAGS[@]}" -ffp-contract=fast -o "$work/fma_algebra.lccc" "$work/fma_algebra.c" 2>"$work/fa.err"
    lccc_rc=$?
    "$ORACLE" -O2 -march=x86-64-v3 -ffp-contract=fast -o "$work/fma_algebra.oracle" "$work/fma_algebra.c" 2>/dev/null
    oracle_rc=$?
    if [[ $lccc_rc -ne 0 ]]; then
        bad "fma_algebra.c did not compile with lccc:"; head -5 "$work/fa.err" | note
    elif [[ $oracle_rc -ne 0 ]]; then
        skipped "reference compiler could not build fma_algebra.c"
    else
        "$work/fma_algebra.lccc" > "$work/fa.lccc" 2>&1; rc_l=$?
        "$work/fma_algebra.oracle" > "$work/fa.oracle" 2>&1; rc_o=$?
        if [[ $rc_l -ne 0 ]]; then
            bad "a contracted FMA kernel does not compute the fused value:"
            grep MISMATCH "$work/fa.lccc" | head -5 | note
        elif [[ $rc_o -ne 0 ]]; then
            skipped "the reference compiler's own contracted kernels are not fused; nothing to compare"
        elif ! diff -q "$work/fa.lccc" "$work/fa.oracle" >/dev/null; then
            bad "builtin FMA output differs from $ORACLE:"
            diff "$work/fa.lccc" "$work/fa.oracle" | head -10 | note
        else
            ok "FMA algebra kernel bit-exact vs $ORACLE on $(grep -c . "$work/fa.lccc") lines; 28 contracted kernels equal their builtin twins"
        fi
        # Anti-vacuity, per encoding: all three forms, a memory operand at
        # operand 0 of a non-231 form, and all four sign families must appear,
        # or the differential did not exercise the re-encoding it checks.
        "$LCCC" "${FLAGS[@]}" -ffp-contract=fast -S -o "$work/fma_algebra.s" "$work/fma_algebra.c" 2>/dev/null
        n132=$(grep -cE '^[[:space:]]*vf(n?m(add|sub))132(sd|ss)' "$work/fma_algebra.s" || true)
        n213=$(grep -cE '^[[:space:]]*vf(n?m(add|sub))213(sd|ss)' "$work/fma_algebra.s" || true)
        n231=$(grep -cE '^[[:space:]]*vf(n?m(add|sub))231(sd|ss)' "$work/fma_algebra.s" || true)
        nmem_non231=$(grep -cE '^[[:space:]]*vf(n?m(add|sub))(132|213)(sd|ss)[[:space:]]+[^%$]' "$work/fma_algebra.s" || true)
        nfam=$(grep -oE '^[[:space:]]*vf(n?m(add|sub))' "$work/fma_algebra.s" | sed 's/^[[:space:]]*//' | sort -u | wc -l)
        # All FOUR sign families must materialize: vfmadd, vfmsub, vfnmadd
        # AND vfnmsub.  The fourth was structurally unreachable before the
        # signed-FMA hook (`-(a*b) - c` kept its vxorpd), so `>= 3` would let
        # a regression that loses vfnmsub pass vacuously.
        if [[ "$n132" -gt 0 && "$n213" -gt 0 && "$n231" -gt 0 && "$nmem_non231" -gt 0 && "$nfam" -eq 4 ]]; then
            ok "the algebra kernel exercises the encodings: $n132 x 132, $n213 x 213, $n231 x 231, $nmem_non231 memory operands on non-231 forms, $nfam sign families"
        else
            bad "the algebra kernel does not exercise every encoding (132=$n132 213=$n213 231=$n231 mem-on-non-231=$nmem_non231 families=$nfam)"
        fi
        # Straight-line kernels that must be ONE instruction plus ret -- GCC's
        # count.  The negation source shapes AND the builtin sign variants are
        # both held to that bar: the IR negation peel (fma_neg_peel.rs)
        # absorbs the operand Negs into the family selection, closing the
        # open item this list used to document (the separate-vxorpd spelling).
        # b_cancel/b_chainneg land on the plain family by sign cancellation;
        # b_named proves non-adjacent negations peel.
        one_insn_ok=1
        for fn in b_madd b_madd_p b_madd_pm b_maddf b_maddf_pm \
                  madd_a msub_a nmadd_a madd_c msub_c nmadd_c madd_m1 madd_m2 madd_ma \
                  sq_add self_add self_add2 maddf_a maddf_m nmaddf_c \
                  neg_prod_sub neg_prod_add neg_sum neg_diff sub_neg_prod neg_neg_sub \
                  neg_prod_subf neg_sumf \
                  b_msub b_nmadd b_nmsub b_cancel b_named b_chainneg b_nmsubf; do
            n=$(fnbody "$work/fma_algebra.s" "$fn" | wc -l)
            if [[ "$n" -ne 2 ]]; then
                one_insn_ok=0
                bad "$fn is $((n - 1)) instructions plus ret, expected exactly one FMA:"
                fnbody "$work/fma_algebra.s" "$fn" | note
            fi
        done
        if [[ "$one_insn_ok" -eq 1 ]]; then
            ok "all 35 straight-line FMA kernels are exactly one instruction plus ret (gcc parity, builtin sign variants included)"
        fi
        # Anti-vacuity for the operand-negation peel: the ONLY materialised
        # negation in any kernel body is the multi-use control's (its Neg
        # feeds the fma AND the return, so single-use discipline must keep
        # it).  main's reference computations also negate (-a, -b_madd) and
        # are outside the kernel bodies by construction.  A peel regression
        # grows the count; a wrongly eager peel that broke the control would
        # shrink it to 0.
        nxor=$(fnbody "$work/fma_algebra.s" b_shared_neg | grep -cE '^vxorp[ds][[:space:]]' || true)
        nxor_all=$(grep -cE '^[[:space:]]*vxorp[ds][[:space:]]' "$work/fma_algebra.s" || true)
        if [[ "$nxor" -eq 1 ]]; then
            ok "exactly one materialised negation survives in the kernels: the multi-use control (operand-negation peel anti-vacuity)"
        else
            bad "b_shared_neg should keep exactly 1 vxorpd (multi-use control), found $nxor of $nxor_all file-wide — the operand-negation peel regressed or over-fired"
            fnbody "$work/fma_algebra.s" b_shared_neg | note
        fi
    fi

    # ── D2. packed builtin FMA loops (MapExpr::Fma) ─────────────────────────
    # The element-wise `__builtin_fma` loop now vectorizes through the map
    # pattern's Fma node — plain AND sign-negated spellings, exactly GCC's
    # packed forms (vfnmsub132pd etc.).  Three assertions: the packed
    # families materialize (anti-vacuity: a regression to the scalar loop
    # shows up as missing pd/ps forms), the runtime output is bit-exact
    # against the reference compiler (both compute per-element fma; the
    # 37-element trip count forces the scalar remainder mirror through
    # every boundary: 36 packed + 1 remainder), and -ffp-contract=off
    # still vectorizes (builtin semantics are not contract-gated — the
    # one deliberate asymmetry against the mul+add contraction).
    cat > "$work/fma_packed.c" <<'EOF_PACKED'
    #include <stdio.h>
    #include <math.h>
    void p_plain(const double *a, const double *b, const double *c, double *r, int n) {
        for (int i = 0; i < n; i++) r[i] = __builtin_fma(a[i], b[i], c[i]);
    }
    void p_both(const double *a, const double *b, const double *c, double *r, int n) {
        for (int i = 0; i < n; i++) r[i] = __builtin_fma(-a[i], b[i], -c[i]);
    }
    void p_prod(const double *a, const double *b, const double *c, double *r, int n) {
        for (int i = 0; i < n; i++) r[i] = __builtin_fma(-a[i], b[i], c[i]);
    }
    void p_addend(const double *a, const double *b, const double *c, double *r, int n) {
        for (int i = 0; i < n; i++) r[i] = __builtin_fma(a[i], b[i], -c[i]);
    }
    void p_f32(const float *a, const float *b, const float *c, float *r, int n) {
        for (int i = 0; i < n; i++) r[i] = __builtin_fmaf(-a[i], b[i], -c[i]);
    }
    void p_inplace(double *a, double *b, double *r, int n) {
        for (int i = 0; i < n; i++) r[i] = __builtin_fma(-a[i], b[i], r[i]);
    }
    int main(void) {
        double a[37], b[37], c[37];
        float fa[37], fb[37], fc[37];
        double xs[] = { 1.0000000000000002, -3.0000000000000004, 0.30000000000000004,
                        1e16, -1e-16, 0.0, -0.0, INFINITY, -INFINITY, 1e308, 1e-308,
                        NAN, -NAN, 2.5, -7.7 };
        int n = (int)(sizeof xs / sizeof *xs);
        for (int i = 0; i < 37; i++) {
            a[i] = xs[i % n]; b[i] = xs[(i + 3) % n]; c[i] = xs[(i + 7) % n];
            fa[i] = (float)xs[i % n]; fb[i] = (float)xs[(i + 3) % n]; fc[i] = (float)xs[(i + 7) % n];
        }
        double r[37]; float fr[37];
        p_plain(a, b, c, r, 37); for (int i = 0; i < 37; i++) printf("%.17g\n", r[i]);
        p_both(a, b, c, r, 37);  for (int i = 0; i < 37; i++) printf("%.17g\n", r[i]);
        p_prod(a, b, c, r, 37);  for (int i = 0; i < 37; i++) printf("%.17g\n", r[i]);
        p_addend(a, b, c, r, 37); for (int i = 0; i < 37; i++) printf("%.17g\n", r[i]);
        p_f32(fa, fb, fc, fr, 37); for (int i = 0; i < 37; i++) printf("%.9g\n", (double)fr[i]);
        p_inplace(a, b, r, 37); for (int i = 0; i < 37; i++) printf("%.17g\n", r[i]);
        return 0;
    }
EOF_PACKED
    "$LCCC" "${FLAGS[@]}" -ffp-contract=fast -o "$work/fp.lccc" "$work/fma_packed.c" 2>/dev/null \
        && "$LCCC" "${FLAGS[@]}" -ffp-contract=fast -S -o "$work/fp.s" "$work/fma_packed.c" 2>/dev/null \
        && "$ORACLE" -O2 -march=x86-64-v3 -ffp-contract=fast -o "$work/fp.oracle" "$work/fma_packed.c" 2>/dev/null
    if [[ $? -ne 0 ]]; then
        bad "fma_packed.c did not build with both compilers"
    else
        "$work/fp.lccc" > "$work/fp.lccc.out" 2>&1; rc_l=$?
        "$work/fp.oracle" > "$work/fp.oracle.out" 2>&1; rc_o=$?
        if [[ $rc_l -ne 0 || $rc_o -ne 0 ]]; then
            bad "packed FMA loop run failed (lccc=$rc_l oracle=$rc_o)"
        elif ! diff -q "$work/fp.lccc.out" "$work/fp.oracle.out" >/dev/null; then
            bad "packed FMA loop output differs from $ORACLE:"
            diff "$work/fp.lccc.out" "$work/fp.oracle.out" | head -6 | note
        else
            ok "packed builtin-FMA loops bit-exact vs $ORACLE (37 elements: packed body + scalar remainder + in-place alias)"
        fi
        # Anti-vacuity: each spelling must materialize its packed family.
        # 132/213/231 all acceptable -- the peephole's form algebra may
        # re-encode -- but the LANE WIDTH (pd/ps) and the FAMILY
        # (madd/msub/nmadd/nmsub) are semantic and pinned.
        want_packed() { # family-re, count-name
            local n
            n=$(grep -cE "^[[:space:]]*$1" "$work/fp.s" || true)
            if [[ "$n" -gt 0 ]]; then ok "packed $2 materialized ($n sites)"; else bad "no packed $2 in fma_packed.s — the Fma map node regressed"; fi
        }
        want_packed 'vfnmsub[0-9]*pd' 'vfnmsub..pd (double-negated loop)'
        want_packed 'vfnmadd[0-9]*pd' 'vfnmadd..pd (product-negated loop)'
        want_packed 'vfmsub[0-9]*pd'  'vfmsub..pd (addend-negated loop)'
        want_packed 'vfmadd[0-9]*pd'  'vfmadd..pd (plain loop)'
        want_packed 'vfnmsub[0-9]*ps' 'vfnmsub..ps (f32 loop)'
        # The contract asymmetry: the builtin loop vectorizes with the
        # contract OFF (C99 semantics), while the mul+add contraction does
        # not -- pinning the one deliberate difference.
        "$LCCC" "${FLAGS[@]}" -ffp-contract=off -S -o "$work/fpco.s" "$work/fma_packed.c" 2>/dev/null
        nco=$(grep -cE '^[[:space:]]*vfmadd[0-9]*pd' "$work/fpco.s" || true)
        if [[ "$nco" -gt 0 ]]; then
            ok "-ffp-contract=off still vectorizes the builtin loop ($nco packed sites)"
        else
            bad "-ffp-contract=off lost the builtin packed FMA — the contract gate leaked into MapExpr::Fma"
        fi
    fi

    # ── E. scalar negation, end to end ──────────────────────────────────────
    # `-x` is now one `vxorpd`/`vxorps` against a rodata sign mask computed in
    # the value's XMM home.  The things that can go wrong are all about bits:
    # the wrong mask (fabs's, one bit off), the wrong lane, a merge into the
    # wrong upper half when the negated register is later read packed, a
    # value that must survive a call, a negated zero whose sign is the whole
    # point, and a NaN whose payload must be preserved.  Bit-exact against the
    # reference compiler, at every optimisation level and with and without
    # AVX, because the emitter has two spellings.
    cat > "$work/negate.c" <<'EOF_NEGATE'
    #include <stdio.h>
    #include <math.h>
    #define NI __attribute__((noinline))
    NI double n_reg(double a) { return -a; }
    NI float  nf_reg(float a) { return -a; }
    NI double n_mem(const double *p) { return -*p; }
    NI float  nf_mem(const float *p) { return -*p; }
    NI double n_zero(double a) { return -(a * 0.0); }
    NI double n_twice(double a) { double t = -a; return -t; }
    NI double n_then_pack(double a, double b) {
        typedef double v2df __attribute__((vector_size(16)));
        double na = -a;
        v2df v = { na, b };
        v = v * v;
        return v[0] + v[1] + na;
    }
    NI double n_loop(const double *xs, int n) {
        double s = 0.0;
        for (int i = 0; i < n; i++) s += -xs[i] * (double)(i + 1);
        return s;
    }
    NI double n_call(double a) { double t = -a; return sin(t) + t; }
    NI double n_cmp(double a, double b) { return (-a < b) ? -a : b; }
    NI double n_mul_sub(double a, double b, double c) { return -(a * b) - c; }
    NI double n_a_mul(double a, double b, double c) { return -a * b - c; }
    NI double n_fma(double a, double b, double c) { return -__builtin_fma(a, b, c); }
    NI float  nf_fma(float a, float b, float c) { return __builtin_fmaf(-a, b, -c); }
    NI long double nld(long double a) { return -a; }
    NI double n_int(int a) { return -(double)a; }
    NI int    n_toint(double a) { return (int)-a; }
    NI double n_select(int s, double a, double b) { return s ? -a : -b; }
    NI double n_many(double a, double b, double c, double d, double e, double f, double g, double h, double i) {
        return -a + -b + -c + -d + -e + -f + -g + -h + -i;
    }
    NI double n_nan_payload(void) {
        union { unsigned long long u; double d; } x = { 0x7ff80000deadbeefULL };
        double n = -x.d;
        union { double d; unsigned long long u; } y = { n };
        return (double)(y.u >> 32) + (double)(y.u & 0xffffffffu);
    }
    int main(void) {
        static const double xs[] = { 1.5, -2.25, 0.0, -0.0, 1e308, -1e-308, 3.0, 7.0 };
        static const float fs[] = { 1.5f, -2.25f, 0.0f, -0.0f, 1e38f, -1e-38f, 3.0f, 7.0f };
        for (int i = 0; i < 8; i++) {
            double a = xs[i], b = xs[(i + 3) % 8], c = xs[(i + 5) % 8];
            printf("%d %.17g %.9g %.17g %.9g %.17g %.17g %.17g %.17g %.17g %.17g %.17g %.17g %.17g %.9g %.21Lg %.17g %d %.17g %.17g\n",
                i, n_reg(a), nf_reg(fs[i]), n_mem(&a), nf_mem(&fs[i]), n_zero(a), n_twice(a),
                n_then_pack(a, b), n_loop(xs, 8), n_call(a), n_cmp(a, b), n_mul_sub(a, b, c), n_a_mul(a, b, c),
                n_fma(a, b, c), nf_fma(fs[i], fs[(i + 3) % 8], fs[(i + 5) % 8]), nld((long double)a), n_int(i - 4),
                n_toint(a > 1e9 ? 3.0 : a), n_select(i & 1, a, b), n_many(a, b, c, a, b, c, a, b, c));
        }
        printf("%.17g\n", n_nan_payload());
        printf("signbit %d %d %d %d\n", __builtin_signbit(n_reg(0.0)), __builtin_signbit(n_reg(-0.0)),
               __builtin_signbit(n_zero(5.0)), __builtin_signbit(n_zero(-5.0)));
        return 0;
    }
EOF_NEGATE
    if "$ORACLE" -O2 -march=x86-64-v3 -o "$work/negate.oracle" "$work/negate.c" -lm 2>/dev/null; then
        "$work/negate.oracle" > "$work/neg.oracle" 2>&1
        neg_ok=1
        for nf in "-O0" "-O1" "-O2" "-O2 -march=x86-64-v2" "-O2 -march=x86-64-v3" "-O3 -march=x86-64-v3" "-O2 -march=x86-64-v3 -ffp-contract=fast"; do
            # shellcheck disable=SC2086
            if ! "$LCCC" $nf -o "$work/negate.lccc" "$work/negate.c" -lm 2>"$work/neg.err"; then
                neg_ok=0; bad "negate.c did not compile with lccc at $nf:"; head -3 "$work/neg.err" | note; continue
            fi
            "$work/negate.lccc" > "$work/neg.lccc" 2>&1
            if ! diff -q "$work/neg.lccc" "$work/neg.oracle" >/dev/null; then
                neg_ok=0; bad "negation output differs from $ORACLE at $nf:"; diff "$work/neg.lccc" "$work/neg.oracle" | head -6 | note
            fi
        done
        if [[ "$neg_ok" -eq 1 ]]; then
            ok "negation kernel bit-exact vs $ORACLE on $(grep -c . "$work/neg.oracle") lines at 7 flag sets (-O0..-O3, v2/v3, contract)"
        fi
        # Anti-vacuity: the v3 build must use the xor form and never the GPR
        # round trip for a scalar negate.
        "$LCCC" -O2 -march=x86-64-v3 -S -o "$work/negate.s" "$work/negate.c" 2>/dev/null
        n_xor=$(grep -cE '^[[:space:]]*vxorp[sd] \.LCFP_[0-9]+\(%rip\)' "$work/negate.s" || true)
        n_gpr=$(grep -cE 'movabsq \$-9223372036854775808|xorl \$0x80000000, %eax' "$work/negate.s" || true)
        if [[ "$n_xor" -ge 20 && "$n_gpr" -eq 0 ]]; then
            ok "every scalar negate is an XMM-domain xor ($n_xor sites, 0 GPR sign-mask round trips)"
        else
            bad "negate lowering: $n_xor xor sites, $n_gpr GPR round trips (expected >= 20 and 0)"
        fi
    else
        skipped "reference compiler could not build negate.c"
    fi

    # AVX-512: the fp liveness oracle treats a writemask destination as a
    # partial write and %zmm as an alias of %xmm.  Only meaningful on a host
    # that has the instructions.
    if grep -qo avx512f /proc/cpuinfo 2>/dev/null; then
        cat > "$work/avx512.c" <<'EOF'
        #include <stdio.h>
        typedef double v8df __attribute__((vector_size(64)));
        __attribute__((noinline)) static v8df masked(v8df a, v8df b) {
            v8df r = a + b;
            return r * a - b;
        }
        int main(void) {
            v8df a = {1.5,-2.5,3.25,-4.125,5.0625,-6.03125,7.5,-8.25};
            v8df b = {0.5,0.25,-0.125,1.0,-1.5,2.0,-2.5,3.0};
            v8df r = masked(a, b);
            double s = 0; for (int i = 0; i < 8; i++) s += r[i];
            printf("%.17g\n", s);
            return 0;
        }
EOF
        if "$LCCC" -O2 -march=x86-64-v3 -mavx512f -o "$work/avx512.lccc" "$work/avx512.c" 2>/dev/null \
           && "$ORACLE" -O2 -march=x86-64-v3 -mavx512f -o "$work/avx512.oracle" "$work/avx512.c" 2>/dev/null; then
            if diff -q <("$work/avx512.lccc") <("$work/avx512.oracle") >/dev/null; then
                ok "AVX-512 kernel bit-exact vs $ORACLE"
            else
                bad "AVX-512 output differs from $ORACLE"
            fi
        else
            skipped "AVX-512 kernel could not be built by both compilers"
        fi
    else
        skipped "host has no avx512f; the EVEX runtime check is not exercised"
    fi
fi

# ── C. corpus ratchet ─────────────────────────────────────────────────────────
# Instruction counts over the FP-heavy slice of the benchmark corpus, measured
# with these passes in place on main at 7b628ba.  A ratchet and not an equality:
# a later improvement should not have to edit this file to land, but a
# regression must fail with the benchmark named.  A ratchet over an empty corpus
# is a passing test that measured nothing, so the number of sources actually
# compiled is asserted too.
declare -A BUDGET=(
    [chacha20_block]=258
    [double_reduction]=135
    [libm_round_family]=207
    [matmul]=126
    [nbody]=312
    [reduction_vecreg]=103
    [sha256_transform]=355
    [spectral_norm]=287
    [struct_copy]=136
    [zlib_ng_adler32]=350
)
measured=0
for name in "${!BUDGET[@]}"; do
    src="tests/benchmark/programs/$name.c"
    [[ -f "$src" ]] || { skipped "no $src"; continue; }
    measured=$((measured+1))
    "$LCCC" "${FLAGS[@]}" -S -o "$work/$name.s" "$src" 2>/dev/null \
        || { bad "$name did not compile"; continue; }
    n=$(insns "$work/$name.s" | grep -cE '\S' || true)
    if [[ "$n" -le "${BUDGET[$name]}" ]]; then
        ok "$name: $n instructions (budget ${BUDGET[$name]})"
    else
        bad "$name regressed to $n instructions (budget ${BUDGET[$name]})"
    fi
done
[[ "$measured" -gt 0 ]] \
    || { echo "FAIL: no benchmark sources found; the ratchet measured nothing" >&2; exit 1; }

echo
echo "vector copy elimination gate: PASS=$pass FAIL=$fail SKIP=$skip"
[[ "$fail" -eq 0 ]] || exit 1
exit 0
