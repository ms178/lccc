#!/usr/bin/env bash
# ============================================================================
# check_loop_idiom.sh — pin loop-idiom recognition decisions.
#
# The differential tests (loop_idiom_copy_basic/norewrite) pin SEMANTICS.
# This script pins the DECISIONS, which are compile-time properties:
#   1. recognised shapes lower to `call memcpy`/`call memmove` (indexed,
#      bump, exit-value, single-block self-loop, and parameter-rooted forms),
#   2. near misses keep their loops (same-root, overlap smear, extra
#      loop-carried state, same-parameter copy),
#   3. the pass is DEFAULT-ON (no env flag needed); CCC_NO_LOOP_IDIOM=1
#      restores the pre-pass codegen; the legacy CCC_LOOP_IDIOM=1 knob is
#      accepted as a no-op.
# ============================================================================
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
CCC=${CCC:-$here/../../target/fastbuild/lccc}
[[ -x $CCC ]] || { echo "check_loop_idiom: lccc not found at $CCC" >&2; exit 1; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

fail=0
compile() { # name, c-file, extra-env... -> $work/name.s
    local name=$1 src=$2; shift 2
    local asm="$work/$name.s"
    if ! env "$@" "$CCC" -O2 -S "$src" -o "$asm" 2>"$work/$name.err"; then
        echo "FAIL($name): compile error"; head -5 "$work/$name.err"; fail=1; return 1
    fi
}
check() { # name, want(1/0), ere-pattern, c-file, extra-env...
    local name=$1 want=$2 pat=$3 src=$4; shift 4
    compile "$name" "$src" "$@" || return 0
    local got=0
    grep -qE "$pat" "$work/$name.s" && got=1
    if [[ $got != "$want" ]]; then
        echo "FAIL($name): want='$pat'=$want got=$got"
        fail=1
    else
        echo "ok($name): '$pat'=$got"
    fi
}
REWRITE='call (memcpy|memmove)'

# --- must rewrite (one TU per shape: rotating one function must not
# --- perturb another's text). Default pipeline: no env flag. ---
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
cat > "$work/r_selfloop.c" <<'EOF'
void f(unsigned char *d, unsigned char *s, unsigned n) {
    unsigned char *p, *q;
    unsigned i;
    for (p = d, q = s, i = 0; i < n; ++i) *p++ = *q++;
}
EOF
cat > "$work/r_param.c" <<'EOF'
void f(unsigned char *d, unsigned char *s, unsigned n) {
    for (unsigned i = 0; i < n; i++) d[i] = s[i];
}
EOF
check r_indexed 1 'call memcpy' "$work/r_indexed.c"
check r_bump    1 'call memcpy' "$work/r_bump.c"
check r_exit    1 'call memcpy' "$work/r_exit.c"
check r_selfloop 1 "$REWRITE"   "$work/r_selfloop.c"
check r_param_move 1 'call memmove' "$work/r_param.c"
check r_param_nomemcpy 0 'call memcpy' "$work/r_param.c"

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
cat > "$work/n_sameparam.c" <<'EOF'
void f(unsigned char *p, unsigned n) {
    for (unsigned i = 0; i < n; i++) p[i] = p[i];
}
EOF
check n_self      0 "$REWRITE" "$work/n_self.c"
check n_smear     0 "$REWRITE" "$work/n_smear.c"
check n_sum       0 "$REWRITE" "$work/n_sum.c"
check n_sameparam 0 "$REWRITE" "$work/n_sameparam.c"

# --- kill switch: CCC_NO_LOOP_IDIOM=1 restores loop codegen ---
check kill_r_indexed 0 "$REWRITE" "$work/r_indexed.c" CCC_NO_LOOP_IDIOM=1
check kill_r_param   0 "$REWRITE" "$work/r_param.c"   CCC_NO_LOOP_IDIOM=1

# --- legacy knob CCC_LOOP_IDIOM=1 accepted as a no-op (still rewrites) ---
check legacy_r_indexed 1 'call memcpy' "$work/r_indexed.c" CCC_LOOP_IDIOM=1

if [[ $fail != 0 ]]; then echo "check_loop_idiom: FAILED"; exit 1; fi
echo "check_loop_idiom: all decisions pinned"
