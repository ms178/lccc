#!/usr/bin/env bash
# AUD-CPU-3: an intrinsic whose instruction-set extension the invocation does
# not permit must be diagnosed, and the same invocation must still emit the
# instruction when it DOES permit it.
#
# Before the fix `-march=x86-64` (SSE2 only) compiled `_mm_crc32_u8` to
# `crc32b` and `_mm_aesenc_si128` to `aesenc`: instructions the declared target
# does not have, i.e. a SIGILL on exactly the hardware the user said to assume,
# with no diagnostic at any stage.  GCC and Clang reject that same source
# ("target specific option mismatch" / "requires target feature"), so the
# verdicts below are checked against GCC wherever GCC has the intrinsic --
# the gate cannot pass by rejecting more than the oracle does.
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
ccc=${CCC:-$repo/target/fastbuild/lccc}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

[ -x "$ccc" ] || { echo "SKIP: $ccc not built" >&2; exit 0; }

fail() { echo "FAIL: $*" >&2; exit 1; }

# 1. Raw-builtin contract: one source per family, rejected exactly when the
#    invocation lacks the extension and lowered when it has it.
cat >"$tmp/crc.c" <<'EOF'
unsigned crc_u8(unsigned c, unsigned char x) { return __builtin_ia32_crc32qi(c, x); }
EOF
cat >"$tmp/aes.c" <<'EOF'
typedef int v4si __attribute__((vector_size(16)));
v4si aes_enc(v4si a, v4si b) { return __builtin_ia32_aesenc128(a, b); }
EOF
cat >"$tmp/clmul.c" <<'EOF'
typedef int v4si __attribute__((vector_size(16)));
v4si clmul(v4si a, v4si b) { return __builtin_ia32_pclmulqdq128(a, b, 0); }
EOF

# reject <source> <flag-expected-in-message> <flags...>
reject() {
    local src=$1 want=$2
    shift 2
    local out rc asm=$tmp/reject-${src%.c}-$(echo "$@" | tr -c 'a-z0-9' '_').s
    rm -f "$asm"
    set +e
    out=$("$ccc" -O2 "$@" -S "$tmp/$src" -o "$asm" 2>&1)
    rc=$?
    set -e
    [ "$rc" -ne 0 ] || fail "$src compiled with $* (rc=0); the instruction needs an extension this target lacks"
    grep -q -- "$want" <<<"$out" ||
        { printf '%s\n' "$out" >&2; fail "$src: the diagnostic does not name $want"; }
    [ ! -s "$asm" ] || fail "$src: assembly was emitted despite the diagnostic"
}

# accept <source> <required-instruction> <flags...>
accept() {
    local src=$1 want=$2
    shift 2
    local asm=$tmp/accept-${src%.c}-$(echo "$@" | tr -c 'a-z0-9' '_').s
    "$ccc" -O2 "$@" -S "$tmp/$src" -o "$asm"
    grep -qE "^[[:space:]]+v?$want" "$asm" ||
        { printf '%s\n' "$(cat "$asm")" >&2;
          fail "$src with $*: expected $want to be emitted"; }
}

reject crc.c -msse4.2 -march=x86-64
accept crc.c crc32 -msse4.2
accept crc.c crc32 -march=x86-64-v2
accept crc.c crc32

reject aes.c -maes -march=x86-64
accept aes.c aesenc -maes
accept aes.c aesenc -march=raptorlake

reject clmul.c -mpclmul -march=x86-64
accept clmul.c pclmulqdq -mpclmul

# The 32-bit target has the same rule and the same escape hatch: i686 has no
# SSE4.2 by default, and `-msse4.2` is what grants it.
reject crc.c -msse4.2 -m32
accept crc.c crc32 -m32 -msse4.2

# 2. Nothing else is affected: a source with no ISA-sensitive intrinsic still
#    compiles at the lowest target level.
cat >"$tmp/plain.c" <<'EOF'
double dot(const double *a, const double *b, int n) {
    double s = 0.0;
    for (int i = 0; i < n; i++) s += a[i] * b[i];
    return s;
}
EOF
"$ccc" -O2 -march=x86-64 -S "$tmp/plain.c" -o "$tmp/plain.s" ||
    fail "a source without ISA-sensitive intrinsics was rejected at -march=x86-64"

# 3. GCC oracle: where GCC ships the same intrinsic, its accept/reject verdict
#    must match ours.  GCC's DEFAULT is x86-64 (v1) while this project's is
#    x86-64-v3, so only the explicit cases are comparable.
gcc_bin=${GCC:-gcc}
if ! command -v "$gcc_bin" >/dev/null 2>&1; then
    echo "SKIP: oracle unavailable ($gcc_bin not found); the contract above still ran" >&2
    echo "PASS: intrinsic feature gate (no oracle on this host)"
    exit 0
fi

cat >"$tmp/o_crc.c" <<'EOF'
#include <nmmintrin.h>
unsigned crc_u8(unsigned c, unsigned char x) { return _mm_crc32_u8(c, x); }
EOF
cat >"$tmp/o_aes.c" <<'EOF'
#include <wmmintrin.h>
__m128i aes_enc(__m128i a, __m128i b) { return _mm_aesenc_si128(a, b); }
EOF

# verdict <source> <flags...>: print "accept" or "reject" for compiler $1
verdict() {
    local cc=$1 src=$2
    shift 2
    if "$cc" -O2 "$@" -c "$tmp/$src" -o /dev/null >/dev/null 2>&1; then
        echo accept
    else
        echo reject
    fi
}

compare() {
    local src=$1
    shift
    local ours theirs
    ours=$(verdict "$ccc" "$src" "$@")
    theirs=$(verdict "$gcc_bin" "$src" "$@")
    [ "$ours" = "$theirs" ] ||
        fail "$src with $*: lccc $ours, gcc $theirs (the oracle is the tie-breaker)"
    echo "ok: $src $* -> $ours (gcc agrees)"
}

compare o_crc.c -march=x86-64
compare o_crc.c -msse4.2
compare o_crc.c -march=x86-64-v2
compare o_aes.c -march=x86-64
compare o_aes.c -maes

echo "PASS: ISA-sensitive intrinsics are gated on the permitted feature set, oracle-checked"
