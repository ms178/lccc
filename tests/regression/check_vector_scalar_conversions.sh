#!/usr/bin/env bash
# Scalar <-> GNU vector conversion diagnostics and ext-vector splat.
#
# Both oracles (GCC 16.2, Clang 23.1.4) reject every scalar <-> plain
# `vector_size` conversion shape, with one documented divergence at the
# tail (same-size vector -> scalar: Clang accepts, GCC rejects; lccc
# implements the GCC verdict):
#   * explicit casts between a vector and a scalar of DIFFERENT size are an
#     error in BOTH directions (GCC: "cannot convert a vector of type
#     '__vector(1) double' to type 'int' which has different size"; the
#     equal-size explicit cast is the documented raw bit reinterpretation
#     and must STAY accepted with the lane's bit pattern visible);
#   * implicit assignment and object initialisation of a scalar into a
#     plain vector (or a vector into a scalar) are errors at ANY size
#     ("incompatible types when assigning"; the same-size vector->scalar
#     leg follows GCC, rejecting Clang's accepted form).
# Before the diagnostic existed, lccc silently accepted these shapes and
# then MISCOMPILED them: the scalar rvalue's value was used as the SOURCE
# ADDRESS of the whole-vector copy and the program faulted at runtime
# (witness: `b = i;` with `vector_size(4)` float emitted `movl (%rax)`
# with %rax == 5). The rejects below are therefore also crash guards.
# The one documented acceptance is the ext-vector splat
# (ext_vector_type = scalar broadcasts to every lane, in both assignment
# and declaration initialisation); its lowering path had the same
# address-confusion class in declaration initialisers and is pinned by a
# running control here.
set -euo pipefail

CCC=${CCC:-./target/fastbuild/lccc}
if [[ ! -x $CCC ]]; then
    echo "FAIL: $CCC not built" >&2
    exit 1
fi
CCC=$(cd "$(dirname -- "$CCC")" && pwd)/$(basename -- "$CCC")

tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-vec-scalar-conv.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
cd "$tmp"

fail() { echo "FAIL: $*" >&2; exit 1; }

hdr='typedef double d1 __attribute__((vector_size(8)));
typedef float f1 __attribute__((vector_size(4)));
typedef float f4 __attribute__((ext_vector_type(4)));'

# case <file> <body statement(s)>
mk() { printf '%s\nint main(void) { %s return 0; }\n' "$hdr" "$2" >"$1"; }

# --- vector_size: explicit casts with mismatched sizes, both directions ----
mk cast_v2s_diffsize.c 'd1 v = {9.7}; int i = (int)v; (void)i;'
mk cast_s2v_diffsize.c 'int i = 5; d1 v = (d1)i; (void)v;'
mk cast_v2s_same_ok_or_miscasts.c 'f1 v = {9.7f}; long l = (long)v; (void)l;'

# --- vector_size: implicit scalar <-> vector at ANY size -------------------
mk init_s2v.c 'd1 v = 9.0; (void)v;'
mk init_s2v_long.c 'long l = 42; d1 v = l; (void)v;'
mk init_v2s.c 'f1 v = {9.7f}; int i = v; (void)i;'
mk assign_s2v_same_size.c 'f1 b = {0.0f}; int i = 5; b = i; (void)b;'
mk assign_s2v_diff_size.c 'd1 v = {0.0}; double d = 2.5; v = d; (void)v;'
mk assign_v2s.c 'f1 v = {9.7f}; int i = 0; i = v; (void)i;'

# expect_rejected <file> <message fragment>
expect_rejected() {
    local file=$1 want=$2
    if "$CCC" -O0 -S "$file" -o "$file.s" 2>"$file.err"; then
        fail "$file was accepted; it must be diagnosed"
    fi
    ! grep -qE "panicked at|internal compiler error|RUST_BACKTRACE" "$file.err" \
        || { cat "$file.err" >&2; fail "$file ICEd instead of diagnosing"; }
    grep -q "$want" "$file.err" \
        || { cat "$file.err" >&2; fail "$file: diagnostic does not mention '$want'"; }
    grep -qE "^${file}:[0-9]+:[0-9]+: error:" "$file.err" \
        || { cat "$file.err" >&2; fail "$file: diagnostic has no file:line:col prefix"; }
    echo "ok: $file -> $(grep -m1 'error:' "$file.err" | sed 's/^[^ ]*//; s/^ //' | cut -c1-90)"
}

expect_rejected cast_v2s_diffsize.c "cannot convert a vector of type"
expect_rejected cast_v2s_diffsize.c "to type 'int' which has different size"
expect_rejected cast_s2v_diffsize.c "cannot convert a value of type 'int' to vector type"
expect_rejected cast_s2v_diffsize.c "which has different size"
expect_rejected cast_v2s_same_ok_or_miscasts.c "which has different size"
expect_rejected init_s2v.c "incompatible types"
expect_rejected init_s2v_long.c "incompatible types"
expect_rejected init_v2s.c "incompatible types"
expect_rejected assign_s2v_same_size.c "incompatible types"
expect_rejected assign_s2v_diff_size.c "incompatible types"
expect_rejected assign_v2s.c "incompatible types"

# --- oracle agreement for every rejected shape ------------------------------
# When the pinned oracle compilers are available, each litmus they reject
# must be one lccc rejects, and each shape they accept must still run.
# Oracle agreement, measured 2026-10-18 against gcc-16.2.0 / clang-23.1.4:
# the seven shapes that leave the same-size lane domain intact are rejected
# by BOTH oracles.  The two SAME-SIZE vector -> scalar shapes
# (init_v2s.c, assign_v2s.c) are a documented GNU/Clang divergence: GCC
# rejects them ("incompatible types") while Clang 23 accepts them as a raw
# reinterpret.  lccc implements the GCC verdict (the GNU vector extension
# defines the semantics this gate pins); Clang is deliberately not
# consulted on those two legs.
shapes_both_oracles_reject="cast_v2s_diffsize.c cast_s2v_diffsize.c \
    cast_v2s_same_ok_or_miscasts.c init_s2v.c init_s2v_long.c \
    assign_s2v_same_size.c assign_s2v_diff_size.c"
if [[ -n ${GCC:-} && -x $GCC ]]; then
    for f in $shapes_both_oracles_reject init_v2s.c assign_v2s.c; do
        if "$GCC" -c "$f" -o /dev/null 2>"$f.gccerr"; then
            fail "$f: the GCC oracle accepts it, but the gate treats it as a reject"
        fi
    done
    echo "ok: GCC oracle rejects all 9 reject shapes (incl. same-size v->s)"
fi
if [[ -n ${CLANG:-} && -x $CLANG ]]; then
    for f in $shapes_both_oracles_reject; do
        if "$CLANG" -c "$f" -o /dev/null 2>"$f.clangerr"; then
            fail "$f: the Clang oracle accepts it, but the gate treats it as a reject"
        fi
    done
    echo "ok: Clang oracle rejects the 7 shared shapes (2 GCC-only rejects skipped:"
    echo "    init_v2s.c / assign_v2s.c -- Clang accepts same-size v->s as raw bits)"
fi

# --- positive controls: legal shapes RUN correctly --------------------------
# 1. Equal-size explicit cast is the raw bit reinterpretation in both
#    directions: (int)(vector_size(4) float{9.7f}) must print the lane's
#    low 32-bit pattern 1092301619, matching both oracles exactly.
cat > rawbits.c <<'EOF'
typedef float f1 __attribute__((vector_size(4)));
int printf(const char *, ...);
int main(void) { f1 v = {9.7f}; int i = (int)v; printf("%d\n", i); return 0; }
EOF
"$CCC" -O0 rawbits.c -o rawbits0 && [ "$(./rawbits0)" = "1092301619" ] \
    || fail "rawbits.c at -O0 produced '$(./rawbits0 2>&1)'; want 1092301619"
"$CCC" -O2 rawbits.c -o rawbits2 && [ "$(./rawbits2)" = "1092301619" ] \
    || fail "rawbits.c at -O2 produced '$(./rawbits2 2>&1)'; want 1092301619"
echo "ok: equal-size explicit vector->int cast reinterprets the lane bits (-O0/-O2)"

# 2. ext-vector splat accepted + lowered: assignment AND declaration init.
cat > splat.c <<'EOF'
typedef float f4 __attribute__((ext_vector_type(4)));
int printf(const char *, ...);
float pick(float a, float b) { return a > b ? a : b; }
int main(void) {
    f4 v = 2.5f;          /* declaration-initialiser splat */
    f4 w;
    w = 2.5f;             /* assignment splat */
    float k = pick(7.25f, 1.0f);
    f4 u = k;             /* non-constant splat */
    printf("%g %g %g %g %g\n",
           (double)v[0], (double)v[3], (double)w[1], (double)u[0], (double)u[3]);
    return 0;
}
EOF
out_expected="2.5 2.5 2.5 7.25 7.25"
"$CCC" -O0 splat.c -o splat0 && [ "$(./splat0)" = "$out_expected" ] \
    || fail "splat at -O0 printed '$(./splat0 2>&1)'; want '$out_expected'"
"$CCC" -O2 splat.c -o splat2 && [ "$(./splat2)" = "$out_expected  " ] || \
    [ "$(./splat2)" = "$out_expected" ] \
    || fail "splat at -O2 printed '$(./splat2 2>&1)'; want '$out_expected'"
echo "ok: ext-vector splat (decl-init, assignment, non-constant) runs (-O0/-O2)"

echo "PASS: scalar <-> vector conversion diagnostics (9 reject shapes, oracle-agreed)"
echo "      plus raw-reinterpret and ext-vector splat positive controls"
