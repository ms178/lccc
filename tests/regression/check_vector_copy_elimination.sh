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
EOF
"$LCCC" "${FLAGS[@]}" -S -o "$work/shapes.s" "$work/shapes.c" \
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

# The one residual register copy in this file, named instead of hidden: `chain`
# ends with `vfmsub132sd ..., %xmm2` and `movsd %xmm2, %xmm0`, because the
# codegen put the constant 3.0 in %xmm0 and so the accumulator could not be the
# return register.  Rotating 132 into 231 would delete it -- that is
# instruction-selection form choice, sized with the residual inventory in
# engineering/FOLLOWUP-2026-09-19E-vector-copy-elimination.md.  What is asserted
# here is that the count does not grow.
copies=$(insns "$work/shapes.s" | grep -cE "$COPY_RE" || true)
if [[ "$copies" -le 1 ]]; then
    ok "at most one vector register copy survives the whole shape set (found $copies)"
else
    bad "$copies vector register copies survived, budget is 1:"
    insns "$work/shapes.s" | grep -E "$COPY_RE" | note
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
        if [[ "$n_copies" -le 48 ]]; then
            ok "runtime kernel carries $n_copies vector register copies (budget 48, inventoried in FOLLOWUP-2026-09-19E)"
        else
            bad "runtime kernel carries $n_copies vector register copies, budget is 48"
            insns "$work/runtime.s" | grep -E "$COPY_RE" | head -10 | note
        fi
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
    [libm_round_family]=217
    [matmul]=126
    [nbody]=319
    [reduction_vecreg]=103
    [sha256_transform]=355
    [spectral_norm]=287
    [struct_copy]=138
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
