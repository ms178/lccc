#!/usr/bin/env bash
# ============================================================================
# check_loop_memset.sh — pin loop-memset recognition decisions.
#
# The differential tests (loop_memset_fill_basic/norewrite) pin SEMANTICS.
# This script pins the DECISIONS, which are compile-time properties:
#   1. recognised fill shapes lower to a memset call (byte, wide-uniform
#      zero, wide-uniform nonzero, advancing pointer, exit-value, nested),
#   2. near misses keep their loops (computed store value, stored load =
#      copy loop, strided store, volatile store),
#   3. the pass is inert under CCC_NO_MEMSET_LOOP=1 (default pipeline
#      bit-identical to pre-pass codegen),
#   4. copy loops never get a memset from THIS pass (disjoint from the
#      loop_idiom memcpy pass),
#   5. the affine model is exact: narrowing index casts, mismatched
#      extensions, narrow unsigned (or -fwrapv signed) index arithmetic and
#      counters that can wrap in their own type keep their loops; scaled,
#      offset, runtime-start, short and second-counter fills rewrite, and
#      i686 accepts what wraps exactly as its 32-bit addresses do. Every
#      decision here equals GCC 16.2's; the wrapping shapes need objects
#      over 4 GiB or -fwrapv overflow to show at run time, so only the
#      decision can be tested.
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
    # shellcheck disable=SC2086 # EXTRA is a flag list
    if ! env "$@" "$CCC" -O2 ${EXTRA:-} -S "$src" -o "$asm" 2>"$work/$name.err"; then
        echo "FAIL($name): compile error"; head -5 "$work/$name.err"; fail=1; return
    fi
    local got=0
    # A memset in tail position is a sibling call: `jmp memset@PLT`.
    grep -Eq "(call|jmp)[[:space:]]+memset" "$asm" && got=1
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

# --- affine exactness (5) ---
cat > "$work/m_runtime_scaled.c" <<'EOF'
void f(int *p, int a, int n) { for (int i = a; i < n; i++) p[i] = 0; }
EOF
cat > "$work/m_offset_signed.c" <<'EOF'
void f(unsigned char *d, int a, int n) { for (int i = a; i < n; i++) d[i + 2] = 0; }
EOF
cat > "$work/m_sub_offset.c" <<'EOF'
void f(unsigned char *d, int a, int n) { for (int i = a; i < n; i++) d[i - 1] = 0; }
EOF
cat > "$work/m_scaled_offset.c" <<'EOF'
void f(int *p, int a, int n) { for (int i = a; i < n; i++) p[i + 3] = 0; }
EOF
cat > "$work/m_short.c" <<'EOF'
void f(unsigned char *d, short a, short n) { for (short i = a; i < n; i++) d[i] = 0; }
EOF
cat > "$work/m_two_signed.c" <<'EOF'
void f(unsigned char *d, int a, int n) { int j = a; for (int i = 0; i < n; i++, j++) d[j] = 0; }
EOF
cat > "$work/m_ne_signed.c" <<'EOF'
void f(unsigned char *d, int a, int n) { for (int i = a; i != n; i++) d[i] = 0; }
EOF
cat > "$work/m_ne_u32_zero.c" <<'EOF'
void f(unsigned char *d, unsigned n) { for (unsigned i = 0; i != n; i++) d[i] = 0; }
EOF
cat > "$work/n_narrow_u8.c" <<'EOF'
void f(unsigned char *d, int n) { for (int i = 0; i < n; i++) d[(unsigned char)i] = 0; }
EOF
cat > "$work/n_narrow_u16.c" <<'EOF'
void f(unsigned char *d, unsigned long n) { for (unsigned long i = 0; i < n; i++) d[(unsigned short)i] = 0; }
EOF
cat > "$work/n_zext_signed.c" <<'EOF'
void f(unsigned char *d, int a, int n) { for (int i = a; i < n; i++) d[(unsigned)i] = 0; }
EOF
cat > "$work/n_u32_offset.c" <<'EOF'
void f(unsigned char *d, unsigned n) { for (unsigned i = 0; i < n; i++) d[i + 2] = 0; }
EOF
cat > "$work/n_ne_u32_wrap.c" <<'EOF'
void f(unsigned char *d, unsigned a, unsigned n) { for (unsigned i = a; i != n; i++) d[i] = 0; }
EOF
cat > "$work/n_ne_short_wrap.c" <<'EOF'
void f(unsigned char *d, short a, short n) { for (short i = a; i != n; i++) d[i] = 0; }
EOF
cat > "$work/n_two_u32.c" <<'EOF'
void f(unsigned char *d, unsigned a, unsigned n) { unsigned j = a; for (unsigned i = 0; i < n; i++, j++) d[j] = 0; }
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
check runtime_scaled   1 "$work/m_runtime_scaled.c"
check offset_signed    1 "$work/m_offset_signed.c"
check sub_offset       1 "$work/m_sub_offset.c"
check scaled_offset    1 "$work/m_scaled_offset.c"
check short_counter    1 "$work/m_short.c"
check two_signed       1 "$work/m_two_signed.c"
check ne_signed        1 "$work/m_ne_signed.c"
check ne_u32_zero      1 "$work/m_ne_u32_zero.c"
check keep_narrow_u8   0 "$work/n_narrow_u8.c"
check keep_narrow_u16  0 "$work/n_narrow_u16.c"
check keep_zext_signed 0 "$work/n_zext_signed.c"
check keep_u32_offset  0 "$work/n_u32_offset.c"
check keep_ne_u32_wrap 0 "$work/n_ne_u32_wrap.c"
check keep_ne_short    0 "$work/n_ne_short_wrap.c"
check keep_two_u32     0 "$work/n_two_u32.c"
EXTRA=-fwrapv check keep_wrapv_offset 0 "$work/m_offset_signed.c"
EXTRA=-fwrapv check keep_wrapv_two    0 "$work/m_two_signed.c"
# i686: 32-bit counters wrap exactly as the 32-bit address arithmetic does.
if "$CCC" -m32 -O2 -S -o /dev/null "$work/m_byte.c" 2>/dev/null; then
    EXTRA=-m32 check i686_u32_offset  1 "$work/n_u32_offset.c"
    EXTRA=-m32 check i686_zext_signed 1 "$work/n_zext_signed.c"
    EXTRA=-m32 check i686_ne_u32      1 "$work/n_ne_u32_wrap.c"
    EXTRA=-m32 check i686_keep_u8     0 "$work/n_narrow_u8.c"
fi

# --- inert under the kill switch: no memset anywhere in a TU that would
# --- otherwise rewrite ---
if grep -q "call memset" "$work/m_byte.s" 2>/dev/null; then :; fi
check inert_kill_switch 0 "$work/m_byte.c" CCC_NO_MEMSET_LOOP=1

# --- pass-name kill switch (CCC_DISABLE_PASSES=loop_memset) ---
check inert_disable_passes 0 "$work/m_byte.c" CCC_DISABLE_PASSES=loop_memset

if [[ $fail != 0 ]]; then echo "check_loop_memset: FAILED"; exit 1; fi
echo "check_loop_memset: all decisions pinned"
