#!/usr/bin/env bash
# ============================================================================
# check_loop_memset.sh — pin loop-memset recognition decisions.
#
# The differential tests (loop_memset_fill_basic/norewrite) pin SEMANTICS.
# This script pins the DECISIONS, which are compile-time properties:
#   1. recognised fill shapes lower to `call memset` (byte, wide-uniform
#      zero, wide-uniform nonzero, advancing pointer, exit-value, nested),
#   2. near misses keep their loops (computed store value, stored load =
#      copy loop, strided store, volatile store),
#   3. the pass is inert under CCC_NO_MEMSET_LOOP=1 (default pipeline
#      bit-identical to pre-pass codegen),
#   4. copy loops never get a memset from THIS pass (disjoint from the
#      loop_idiom memcpy pass).
# ============================================================================
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
CCC=${CCC:-$here/../../target/fastbuild/lccc}
[[ -x $CCC ]] || { echo "check_loop_memset: lccc not found at $CCC" >&2; exit 1; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

fail=0
check() { # name, want_memset(1/0), c-file, extra-env...
    local name=$1 want=$2 src=$3; shift 3
    local asm="$work/$name.s"
    if ! env "$@" "$CCC" -O2 -S "$src" -o "$asm" 2>"$work/$name.err"; then
        echo "FAIL($name): compile error"; head -5 "$work/$name.err"; fail=1; return
    fi
    local got=0
    grep -q "call memset" "$asm" && got=1
    if [[ $got != "$want" ]]; then
        echo "FAIL($name): want_memset=$want got=$got"
        fail=1
    else
        echo "ok($name): memset=$got"
    fi
}

# --- must rewrite (one TU per shape: rotating one function must not
# --- perturb another's text) ---
cat > "$work/m_byte.c" <<'EOF'
unsigned char G[4096]; unsigned N;
void f(void) { for (unsigned i = 0; i < N; i++) G[i] = 0; }
EOF
cat > "$work/m_byte_nz.c" <<'EOF'
unsigned char G[4096]; unsigned N;
void f(void) { for (unsigned i = 0; i < N; i++) G[i] = 0xA7; }
EOF
cat > "$work/m_wide_zero.c" <<'EOF'
unsigned int W[1024]; unsigned N;
void f(void) { for (unsigned i = 0; i < N; i++) W[i] = 0u; }
EOF
cat > "$work/m_wide_pattern.c" <<'EOF'
unsigned int W[1024]; unsigned N;
void f(void) { for (unsigned i = 0; i < N; i++) W[i] = 0xAAAAAAAAu; }
EOF
cat > "$work/m_bump.c" <<'EOF'
unsigned char G[4096]; unsigned N;
void f(void) { unsigned char *d = G; for (unsigned i = 0; i < N; i++) *d++ = 0; }
EOF
cat > "$work/m_exituse.c" <<'EOF'
unsigned char G[4096]; unsigned N; unsigned char *after;
void f(void) { unsigned char *d = G; for (unsigned i = 0; i < N; i++) *d++ = 0; after = d; }
EOF
cat > "$work/m_nested.c" <<'EOF'
unsigned char G[4096]; unsigned N;
void f(unsigned k) { for (unsigned i = 0; i < N; i++) for (unsigned j = 0; j < k; j++) G[j] = 0; }
EOF
cat > "$work/m_u64_for.c" <<'EOF'
unsigned char G[4096]; unsigned long N;
void f(void) { for (unsigned long i = 0; i < N; i++) G[i] = 0; }
EOF
cat > "$work/m_u64_while.c" <<'EOF'
unsigned char G[4096]; unsigned long N;
void f(void) { unsigned long i = 0; while (i < N) { G[i] = 0; i++; } }
EOF

# --- must NOT rewrite ---
cat > "$work/n_computed.c" <<'EOF'
unsigned char G[4096]; unsigned N;
void f(void) { for (unsigned i = 0; i < N; i++) G[i] = (unsigned char)i; }
EOF
cat > "$work/n_copy.c" <<'EOF'
unsigned char G[4096], S[4096]; unsigned N;
void f(void) { for (unsigned i = 0; i < N; i++) G[i] = S[i]; }
EOF
cat > "$work/n_strided.c" <<'EOF'
unsigned int W[2048];
void f(void) { for (unsigned i = 0; i < 256; i++) W[i * 2] = 0u; }
EOF
cat > "$work/n_volatile.c" <<'EOF'
unsigned char G[4096]; unsigned N;
void f(void) { for (unsigned i = 0; i < N; i++) *(volatile unsigned char *)(G + i) = 0; }
EOF

# --- decisions ---
check byte_fill        1 "$work/m_byte.c"
check byte_fill_nz     1 "$work/m_byte_nz.c"
check wide_zero        1 "$work/m_wide_zero.c"
check wide_pattern     1 "$work/m_wide_pattern.c"
check bump_fill        1 "$work/m_bump.c"
check exit_value_fill  1 "$work/m_exituse.c"
check nested_fill      1 "$work/m_nested.c"
check u64_for_fill     1 "$work/m_u64_for.c"
check u64_while_fill   1 "$work/m_u64_while.c"

check keep_computed    0 "$work/n_computed.c"
check keep_copy        0 "$work/n_copy.c"
check keep_strided     0 "$work/n_strided.c"
check keep_volatile    0 "$work/n_volatile.c"

# --- inert under the kill switch: no memset anywhere in a TU that would
# --- otherwise rewrite ---
if grep -q "call memset" "$work/m_byte.s" 2>/dev/null; then :; fi
check inert_kill_switch 0 "$work/m_byte.c" CCC_NO_MEMSET_LOOP=1

# --- pass-name kill switch (CCC_DISABLE_PASSES=loop_memset) ---
check inert_disable_passes 0 "$work/m_byte.c" CCC_DISABLE_PASSES=loop_memset

if [[ $fail != 0 ]]; then echo "check_loop_memset: FAILED"; exit 1; fi
echo "check_loop_memset: all decisions pinned"
