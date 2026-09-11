#!/usr/bin/env bash
# ============================================================================
# check_loop_idiom.sh — pin loop-idiom recognition decisions.
#
# The differential tests (loop_idiom_copy_basic/norewrite) pin SEMANTICS.
# This script pins the DECISIONS, which are compile-time properties:
#   1. recognised shapes lower to `call memcpy` (indexed, bump, nested,
#      const-bound, wide-IV, exit-value forms),
#   2. near misses keep their loops (same-root, overlap smear, extra
#      loop-carried state, parameter roots, two-condition test),
#   3. the pass is inert without CCC_LOOP_IDIOM=1 (default pipeline
#      bit-identical).
# ============================================================================
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
CCC=${CCC:-$here/../../target/fastbuild/lccc}
[[ -x $CCC ]] || { echo "check_loop_idiom: lccc not found at $CCC" >&2; exit 1; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

fail=0
check() { # name, want_memcpy(1/0), c-file, extra-env...
    local name=$1 want=$2 src=$3; shift 3
    local asm="$work/$name.s"
    if ! env "$@" "$CCC" -O2 -S "$src" -o "$asm" 2>"$work/$name.err"; then
        echo "FAIL($name): compile error"; head -5 "$work/$name.err"; fail=1; return
    fi
    local got=0
    grep -q "call memcpy" "$asm" && got=1
    if [[ $got != "$want" ]]; then
        echo "FAIL($name): want_memcpy=$want got=$got"
        fail=1
    else
        echo "ok($name): memcpy=$got"
    fi
}

# --- must rewrite (one TU per shape: rotating one function must not
# --- perturb another's text) ---
cat > "$work/r_indexed.c" <<'EOF'
unsigned char G1[4096], G2[4096];
unsigned LEN;
void f(void) { for (unsigned i = 0; i < LEN; i++) G2[i] = G1[i]; }
EOF
cat > "$work/r_bump.c" <<'EOF'
unsigned char A[1024], B[1024];
unsigned N;
void f(void) {
    unsigned char *d = B, *s = A;
    for (unsigned i = 0; i < N; i++) *d++ = *s++;
}
EOF
cat > "$work/r_exit.c" <<'EOF'
unsigned char A[1024], B[1024];
unsigned N;
unsigned char *after;
void f(void) {
    unsigned char *d = B;
    for (unsigned i = 0; i < N; i++) *d++ = A[i];
    after = d;
}
EOF
check r_indexed 1 "$work/r_indexed.c" CCC_LOOP_IDIOM=1
check r_bump    1 "$work/r_bump.c"    CCC_LOOP_IDIOM=1
check r_exit    1 "$work/r_exit.c"    CCC_LOOP_IDIOM=1

# --- must NOT rewrite ---
cat > "$work/n_self.c" <<'EOF'
unsigned char G1[512];
void f(unsigned n) { for (unsigned i = 0; i < n; i++) G1[i] = G1[i]; }
EOF
cat > "$work/n_smear.c" <<'EOF'
unsigned char G1[512];
void f(unsigned n) { for (unsigned i = 0; i < n; i++) G1[i + 1] = G1[i]; }
EOF
cat > "$work/n_sum.c" <<'EOF'
unsigned char G1[512], G2[512];
unsigned N, sum;
void f(void) {
    unsigned s = 0;
    for (unsigned i = 0; i < N; i++) { G2[i] = G1[i]; s += G1[i]; }
    sum = s;
}
EOF
cat > "$work/n_param.c" <<'EOF'
void f(unsigned char *d, unsigned char *s, unsigned n) {
    for (unsigned i = 0; i < n; i++) d[i] = s[i];
}
EOF
check n_self  0 "$work/n_self.c"  CCC_LOOP_IDIOM=1
check n_smear 0 "$work/n_smear.c" CCC_LOOP_IDIOM=1
check n_sum   0 "$work/n_sum.c"   CCC_LOOP_IDIOM=1
check n_param 0 "$work/n_param.c" CCC_LOOP_IDIOM=1

# --- default pipeline inert: without the opt-in flag no TU may change ---
for t in r_indexed r_bump r_exit n_self n_smear n_sum n_param; do
    check "off_$t" 0 "$work/$t.c"
done

if [[ $fail != 0 ]]; then echo "check_loop_idiom: FAILED"; exit 1; fi
echo "check_loop_idiom: all decisions pinned"
