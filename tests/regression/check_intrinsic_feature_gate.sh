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

# The 256-bit AES/CLMUL forms.  They share the mnemonic with the 128-bit forms
# and are a DIFFERENT extension: `vaesenc %ymm1, %ymm0, %ymm0` came out of
# `-march=x86-64` before the fix below, i.e. a SIGILL on the hardware the user
# named.  Their permission is a conjunction -- 256-bit vector class AND the
# extension's own `-m` flag -- so the rejection cases below split along both
# halves, and both are oracle-checked against GCC further down.
cat >"$tmp/vaes256.c" <<'EOF'
typedef long long v4di __attribute__((vector_size(32)));
v4di aes_enc256(v4di a, v4di b) { return __builtin_ia32_aesenc256(a, b); }
EOF
cat >"$tmp/vpclmul256.c" <<'EOF'
typedef long long v4di __attribute__((vector_size(32)));
v4di clmul256(v4di a, v4di b) { return __builtin_ia32_vpclmulqdq256(a, b, 0); }
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
reject crc.c -msse4.2   # default x86-64 baseline has no SSE4.2 (GCC rejects too)

reject aes.c -maes -march=x86-64
accept aes.c aesenc -maes
accept aes.c aesenc -march=raptorlake

reject clmul.c -mpclmul -march=x86-64
accept clmul.c pclmulqdq -mpclmul

# (the second argument of `reject` is the text the diagnostic must contain;
#  every rejection names the `-m` flag that would have granted the extension.)
reject vaes256.c -mvaes -march=x86-64             # v1: neither half
reject vaes256.c -mvaes -march=x86-64-v3          # class yes, extension no
reject vaes256.c -mvaes -march=x86-64-v3 -mno-avx2  # extension yes, class no
reject vaes256.c -mvaes -march=x86-64-v3 -mno-avx   # no VEX at all
reject vaes256.c -mvaes -march=alderlake -mno-vaes  # explicit denial of the profile's grant
accept vaes256.c vaesenc -mvaes                   # -mvaes implies AVX (VEX.256 class), not AVX2
accept vaes256.c vaesenc -march=x86-64-v3 -mno-avx2 -mvaes   # GCC accepts: AVX class only
reject vaes256.c -mvaes -mno-avx                  # AVX denial removes the class
accept vaes256.c vaesenc -march=alderlake

reject vpclmul256.c -mvpclmulqdq -march=x86-64
reject vpclmul256.c -mvpclmulqdq -march=x86-64-v3
reject vpclmul256.c -mvpclmulqdq -march=alderlake -mno-vpclmulqdq
# VAES is not VPCLMULQDQ: separate bits, separate rows (GCC rejects this too).
reject vpclmul256.c -mvpclmulqdq -mvaes -march=x86-64-v3
reject vpclmul256.c -mvpclmulqdq -mvaes -march=alderlake -mno-vpclmulqdq
accept vpclmul256.c vpclmulqdq -mvpclmulqdq
accept vpclmul256.c vpclmulqdq -march=alderlake

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

# Native packed integer operations are not baseline SSE2 merely because
# their C arguments fit in 128 bits. Raw and public names share permission.
cat >"$tmp/o_ssse3.c" <<'EOF'
#include <tmmintrin.h>
__m128i sub(__m128i a,__m128i b) { return _mm_hsub_epi32(a,b); }
EOF
cat >"$tmp/o_sse41.c" <<'EOF'
#include <smmintrin.h>
__m128i mul(__m128i a,__m128i b) { return _mm_mullo_epi32(a,b); }
EOF
reject o_ssse3.c -mssse3 -march=x86-64
reject o_ssse3.c -mssse3 -mssse3 -mno-ssse3
accept o_ssse3.c phsubd -march=x86-64 -mssse3
accept o_ssse3.c phsubd -mno-ssse3 -mssse3
reject o_sse41.c -msse4.1 -march=x86-64
accept o_sse41.c pmulld -march=x86-64 -msse4.1
cat >"$tmp/o_local_ssse3.c" <<'EOF'
#include <tmmintrin.h>
__attribute__((target("ssse3"))) __m128i f(__m128i a,__m128i b) { return _mm_hsub_epi32(a,b); }
EOF
cat >"$tmp/o_local_sse41.c" <<'EOF'
#include <smmintrin.h>
__attribute__((target("sse4.1"))) __m128i f(__m128i a,__m128i b) { return _mm_mullo_epi32(a,b); }
EOF
accept o_local_ssse3.c phsubd -march=x86-64
accept o_local_sse41.c pmulld -march=x86-64
compare o_local_ssse3.c -march=x86-64
compare o_local_sse41.c -march=x86-64
# A local grant must not make an unrelated baseline function legal.
cat "$tmp/o_local_ssse3.c" > "$tmp/o_local_no_leak.c"
printf '%s\n' '__m128i g(__m128i a,__m128i b) { return _mm_hsub_epi32(a,b); }' >> "$tmp/o_local_no_leak.c"
reject o_local_no_leak.c -mssse3 -march=x86-64
compare o_local_no_leak.c -march=x86-64
compare o_ssse3.c -march=x86-64
compare o_ssse3.c -march=x86-64 -mssse3
compare o_ssse3.c -mssse3 -mno-ssse3
compare o_sse41.c -march=x86-64
compare o_sse41.c -march=x86-64 -msse4.1

compare o_crc.c -march=x86-64
compare o_crc.c -msse4.2
compare o_crc.c -march=x86-64-v2
compare o_aes.c -march=x86-64
compare o_aes.c -maes

# The 256-bit rows, same oracle rule.  `-march=alderlake` (not `raptorlake`:
# GCC 12 does not know that name) is the CPU profile both compilers agree on.
cat >"$tmp/o_vaes256.c" <<'EOF'
#include <immintrin.h>
__m256i aes_enc256(__m256i a, __m256i b) { return _mm256_aesenc_epi128(a, b); }
EOF
cat >"$tmp/o_vpclmul256.c" <<'EOF'
#include <immintrin.h>
__m256i clmul256(__m256i a, __m256i b) { return _mm256_clmulepi64_epi128(a, b, 0); }
EOF

compare o_vaes256.c -march=x86-64
compare o_vaes256.c -march=x86-64-v3
compare o_vaes256.c -march=x86-64-v3 -mvaes
compare o_vaes256.c -march=x86-64-v3 -mvaes -mno-avx
compare o_vaes256.c -march=alderlake
compare o_vaes256.c -march=alderlake -mno-vaes
compare o_vpclmul256.c -march=x86-64-v3
compare o_vpclmul256.c -march=x86-64-v3 -mvaes
compare o_vpclmul256.c -march=x86-64-v3 -mvpclmulqdq
compare o_vpclmul256.c -march=alderlake

# VAES and VPCLMULQDQ at 256 bits are VEX.256 forms whose class is AVX, not
# AVX2 (measured gcc 14: `-mvaes -mno-avx2` and bare `-mvaes` accept the
# 256-bit intrinsic; `-mvaes -mno-avx` rejects). The extension flags also
# imply AVX, as GCC defines __AVX__ for them. These are exact GCC comparisons.
compare o_vaes256.c -march=x86-64-v3 -mvaes -mno-avx2
compare o_vaes256.c -mvaes
compare o_vaes256.c -march=x86-64 -mvaes
compare o_vaes256.c -march=x86-64-v3 -mvaes -mno-avx
compare o_vpclmul256.c -mvpclmulqdq
compare o_vpclmul256.c -march=x86-64 -mvpclmulqdq

# ---------------------------------------------------------------------------
# AVX-VNNI, AVX-VNNI-INT8 and AVX-VNNI-INT16: three extensions, three bits,
# three flags.  This is where the audit's live SIGILL case lived: measured
# emitting `vpdpbusd %ymm1, %ymm0, %ymm0` for `_mm_dpbusd_epi32` under
# `-march=x86-64`, an Alder-Lake-era instruction inside a v1 object.
cat >"$tmp/vnni128.c" <<'EOF'
typedef int v4si __attribute__((vector_size(16)));
v4si dpbusd128(v4si a, v4si b, v4si c) { return __builtin_ia32_vpdpbusd128(a, b, c); }
EOF
cat >"$tmp/vnni256.c" <<'EOF'
typedef int v8si __attribute__((vector_size(32)));
v8si dpbusd256(v8si a, v8si b, v8si c) { return __builtin_ia32_vpdpbusd256(a, b, c); }
EOF
cat >"$tmp/vnni_int8.c" <<'EOF'
typedef int v4si __attribute__((vector_size(16)));
v4si dpbssd128(v4si a, v4si b, v4si c) { return __builtin_ia32_vpdpbssd128(a, b, c); }
EOF
cat >"$tmp/vnni_int16.c" <<'EOF'
typedef int v4si __attribute__((vector_size(16)));
v4si dpwusd128(v4si a, v4si b, v4si c) { return __builtin_ia32_vpdpwusd128(a, b, c); }
EOF

# Both halves of the conjunction, on both widths: no vector class (v1), no
# request (v3), no class (v3 -mno-avx2), an explicit denial of a request that
# the flag itself just made, and -- the row that keeps the families apart --
# AVX-VNNI's flag must NOT grant the INT8/INT16 instructions.
reject vnni128.c -mavxvnni -march=x86-64
reject vnni256.c -mavxvnni -march=x86-64
reject vnni128.c -mavxvnni -march=x86-64-v3
reject vnni256.c -mavxvnni -march=x86-64-v3
reject vnni256.c -mavxvnni -march=x86-64-v3 -mno-avx2
reject vnni256.c -mavxvnni -mavxvnni -mno-avxvnni
reject vnni128.c -mavxvnni -march=alderlake -mno-avxvnni
reject vnni_int8.c -mavxvnniint8 -march=x86-64
reject vnni_int8.c -mavxvnniint8 -mavxvnni
reject vnni_int8.c -mavxvnniint8 -march=alderlake
reject vnni_int16.c -mavxvnniint16 -mavxvnni
reject vnni_int16.c -mavxvnniint16 -march=alderlake
accept vnni128.c vpdpbusd -mavxvnni
accept vnni256.c vpdpbusd -mavxvnni
accept vnni128.c vpdpbusd -march=alderlake
accept vnni256.c vpdpbusd -mavx512vnni -mavx512vl
accept vnni_int8.c vpdpbssd -mavxvnniint8
accept vnni_int16.c vpdpwusd -mavxvnniint16
accept vnni128.c vpdpbusd -mavxvnni -mno-avxvnni -mavxvnni  # re-enabled after denial

# The i686 driver shares the flag spelling and rejects without it -- but its
# fold reaches the intrinsic table through `x86_isa()`, which is `NONE` for
# anything but x86-64 (by design: the 32-bit backend does not route its
# *scalar/SSE* work through that model), so the 256-bit VEX intrinsics are
# refused there even when `-mavx -mavxvnni` asks for them.  Refusing is the safe
# direction and the row below pins it; the inconsistent half -- the `__m256`
# *vector-op* path emitting `%ymm` on i686 with no permission at all -- is
# latched separately as AUD-CPU-4 at the bottom of this gate.
reject vnni128.c -mavxvnni -m32 -march=i686
reject vnni128.c -mavxvnni -m32 -msse2 -mavx -mavxvnni

# GCC oracle for the classic AVX-VNNI group (`-mavxvnni` exists in GCC 12).
# The INT8/INT16 halves get no oracle on this toolchain and are declared as
# such below rather than compared against a compiler that cannot express them.
cat >"$tmp/o_vnni128.c" <<'EOF'
#include <immintrin.h>
__m128i dpbusd128(__m128i a, __m128i b, __m128i c) { return _mm_dpbusd_epi32(a, b, c); }
EOF
cat >"$tmp/o_vnni256.c" <<'EOF'
#include <immintrin.h>
__m256i dpbusd256(__m256i a, __m256i b, __m256i c) { return _mm256_dpbusd_epi32(a, b, c); }
EOF

compare o_vnni128.c -march=x86-64
compare o_vnni256.c -march=x86-64
compare o_vnni128.c -march=x86-64-v3
compare o_vnni256.c -march=x86-64-v3
compare o_vnni128.c -mavxvnni
compare o_vnni256.c -mavxvnni
compare o_vnni256.c -mavxvnni -mno-avx
compare o_vnni256.c -mavxvnni -mno-avx2
compare o_vnni128.c -mavx512vnni
compare o_vnni256.c -mavx512vnni
compare o_vnni128.c -mavx512vnni -mavx512vl
compare o_vnni256.c -mavx512vnni -mavx512vl
compare o_vnni128.c -march=alderlake
compare o_vnni256.c -march=alderlake

# `-mavxvnni` implies AVX2 (GCC 14 accepts the 256-bit form under
# `-march=x86-64 -mavxvnni` and rejects it under `-mavxvnni -mno-avx2`), so the
# 256-bit class is granted by the extension itself, not by the v1 baseline.
ours=$(verdict "$ccc" o_vnni128.c -march=x86-64 -mavxvnni)
[ "$ours" = accept ] ||
    fail "o_vnni128.c -march=x86-64 -mavxvnni: lccc $ours, expected accept (-mavxvnni implies AVX2, as GCC 14 accepts)"
echo "ok: o_vnni128.c -march=x86-64 -mavxvnni -> accept (gcc agrees: -mavxvnni implies AVX2)"

# Declared no-oracle region: GCC 12.2 neither knows `-mavxvnniint8` /
# `-mavxvnniint16` (measured: "unrecognized command-line option") nor declares
# those intrinsics in its headers, so there is nothing to compare against.  The
# rows above still pin the contract on the lccc side, and the two families are
# accepted by no flag but their own.
if "$gcc_bin" -mavxvnniint8 -E - </dev/null >/dev/null 2>&1; then
    cat >"$tmp/o_vnni_int8.c" <<'EOF'
#include <immintrin.h>
__m128i dpbssd128(__m128i a, __m128i b, __m128i c) { return _mm_dpbssd_epi32(a, b, c); }
EOF
    compare o_vnni_int8.c -mavxvnniint8
else
    echo "note: no GCC oracle for AVX-VNNI-INT8/-INT16 on this toolchain ($gcc_bin rejects the flag); the lccc contract rows above are the whole gate for those two families"
fi

# ---------------------------------------------------------------------------
# GFNI: the Galois-field byte instructions.  Legacy-encoded (no VEX), so this
# is the one extension whose permission is the flag ALONE -- `-mno-avx` must
# not remove it, and, measured before the row below existed, `-march=x86-64`
# emitted `gf2p8mulb %xmm1, %xmm0` with no diagnostic.  GCC rejects that same
# source, and GCC 12.2 knows `-mgfni`, so every row here is oracle-checked.
cat >"$tmp/gfni.c" <<'EOF'
typedef char v16qi __attribute__((vector_size(16)));
v16qi gmul(v16qi a, v16qi b) { return __builtin_ia32_gf2p8mulb(a, b); }
EOF

reject gfni.c -mgfni -march=x86-64
reject gfni.c -mgfni -march=x86-64-v3
reject gfni.c -mgfni -maes
reject gfni.c -mgfni -march=alderlake -mno-gfni
accept gfni.c gf2p8mulb -mgfni
accept gfni.c gf2p8mulb -mgfni -mavx
accept gfni.c gf2p8mulb -mgfni -mno-avx
accept gfni.c gf2p8mulb -march=alderlake

cat >"$tmp/o_gfni.c" <<'EOF'
#include <immintrin.h>
__m128i gmul(__m128i a, __m128i b) { return _mm_gf2p8mul_epi8(a, b); }
EOF
compare o_gfni.c -march=x86-64
compare o_gfni.c -march=x86-64-v3
compare o_gfni.c -mgfni
compare o_gfni.c -mgfni -mavx
compare o_gfni.c -mgfni -mno-avx
compare o_gfni.c -march=alderlake
compare o_gfni.c -maes
compare o_gfni.c -march=alderlake -mno-gfni

# AUD-CPU-4: YMM intrinsic permission must agree with GCC on both ABIs.
cat >"$tmp/i686_ymm_float.c" <<'EOF'
#include <immintrin.h>
__m256 add256(__m256 a, __m256 b) { return _mm256_add_ps(a, b); }
EOF
cat >"$tmp/i686_ymm_int.c" <<'EOF'
#include <immintrin.h>
__m256i addi256(__m256i a, __m256i b) { return _mm256_add_epi32(a, b); }
EOF
# Regression for the former AUD-CPU-4 SIGILL debt: require the same
# rejection and permitted native forms as GCC in both x86 modes.
for mode in -m32 -m64; do
    reject i686_ymm_float.c -mavx "$mode" -mno-avx
    reject i686_ymm_int.c -mavx2 "$mode" -mavx -mno-avx2
    accept i686_ymm_float.c vaddps "$mode" -mavx -mno-avx2
    accept i686_ymm_int.c vpaddd "$mode" -mavx2
    compare i686_ymm_float.c "$mode" -mno-avx
    compare i686_ymm_int.c "$mode" -mavx -mno-avx2
    compare i686_ymm_float.c "$mode" -mavx -mno-avx2
    compare i686_ymm_int.c "$mode" -mavx2
done

# Scalar-count packed shifts retain their ISA requirement even though their
# runtime count uses an XMM register rather than an immediate.
cat >"$tmp/shift128.c" <<'EOF'
#include <immintrin.h>
__m128i shift(__m128i a, int n) { return _mm_slli_epi16(a, n); }
EOF
cat >"$tmp/shift256.c" <<'EOF'
#include <immintrin.h>
__m256i shift(__m256i a, int n) { return _mm256_slli_epi16(a, n); }
EOF
for mode in -m32 -m64; do
    reject shift128.c -msse2 "$mode" -mno-sse2 -msse
    accept shift128.c psllw "$mode" -msse2
    reject shift256.c -mavx2 "$mode" -mavx -mno-avx2
    accept shift256.c vpsllw "$mode" -mavx2
    compare shift128.c "$mode" -mno-sse2 -msse
    compare shift128.c "$mode" -msse2
    compare shift256.c "$mode" -mavx -mno-avx2
    compare shift256.c "$mode" -mavx2
done

# All VTEST scalar flags require AVX even at 128 bits; none require AVX2.
# Compile-only raw vector sources need no target libc headers in either mode.
for bits in 128 256; do
    for kind in ps pd; do
        element=float
        [ "$kind" = pd ] && element=double
        suffix=
        [ "$bits" = 256 ] && suffix=256
        for flag in z c nzc; do
            src=vtest-$bits-$kind-$flag.c
            cat >"$tmp/$src" <<EOF
typedef $element vec __attribute__((vector_size($((bits / 8)))));
int test(vec a, vec b) { return __builtin_ia32_vtest$flag$kind$suffix(a, b); }
EOF
            for mode in -m64 -m32; do
                reject "$src" -mavx "$mode" -mavx -mno-avx
                accept "$src" "vtest$kind" "$mode" -mavx -mno-avx2
            done
        done
    done
done

# Widening instructions are AVX2, unlike floating-point VTEST. Even when
# only four low source bytes participate, the C input remains a 128-bit value.
for sign in sx zx; do
    for shape in bw bd bq wd wq dq; do
        input=char
        output=short
        case "$shape" in
            wd|wq) input=short ;;
            dq) input=int ;;
        esac
        case "$shape" in
            bd|wd) output=int ;;
            bq|wq|dq) output='long long' ;;
        esac
        src=widen-$sign-$shape.c
        cat >"$tmp/$src" <<EOF
typedef $input source __attribute__((vector_size(16)));
typedef $output result __attribute__((vector_size(32)));
result widen(source a) { return __builtin_ia32_pmov$sign${shape}256(a); }
EOF
        for mode in -m64 -m32; do
            reject "$src" -mavx2 "$mode" -mavx -mno-avx2
            accept "$src" "vpmov$sign$shape" "$mode" -mavx2
        done
    done
done

# Native builtins must reject scalar or wrong-width operands before codegen.
cat >"$tmp/vtest_wrong_shape.c" <<'EOF'
typedef float v4sf __attribute__((vector_size(16)));
int wrong(v4sf a) { return __builtin_ia32_vtestzps256(a, a); }
EOF
cat >"$tmp/widen_wrong_shape.c" <<'EOF'
typedef char v32qi __attribute__((vector_size(32)));
typedef short v16hi __attribute__((vector_size(32)));
v16hi wrong(v32qi a) { return __builtin_ia32_pmovsxbw256(a); }
EOF
for mode in -m64 -m32; do
    reject vtest_wrong_shape.c 'requires two' "$mode" -mavx
    reject widen_wrong_shape.c 'requires a 128-bit vector' "$mode" -mavx2
done

# AVX-512 permissions follow the architectural subset conjunction, including
# EVEX.128/256 VL encodings. The test operations are public wrappers so this
# checks the same generic builtin path real headers use.
cat >"$tmp/avx512_bw_vl.c" <<'EOF'
#include <immintrin.h>
__mmask16 cmp_u8(__m128i a, __m128i b) { return _mm_cmp_epu8_mask(a, b, _MM_CMPINT_EQ); }
EOF
cat >"$tmp/avx512_ternlog_vl.c" <<'EOF'
#include <immintrin.h>
__m128i tern(__m128i a, __m128i b, __m128i c) { return _mm_ternarylogic_epi32(a, b, c, 0x96); }
EOF
cat >"$tmp/avx512_maskload_bw_vl.c" <<'EOF'
#include <immintrin.h>
__m256i load(const void *p, __mmask32 k) { return _mm256_maskz_loadu_epi8(k, p); }
EOF
cat >"$tmp/avx512_f_qword.c" <<'EOF'
#include <immintrin.h>
__m512i absq(__m512i a) { return _mm512_abs_epi64(a); }
EOF
cat >"$tmp/avx512_bitalg.c" <<'EOF'
#include <immintrin.h>
__m512i pop8(__m512i a) { return _mm512_popcnt_epi8(a); }
EOF
cat >"$tmp/avx512_vpopcnt.c" <<'EOF'
#include <immintrin.h>
__m512i pop32(__m512i a) { return _mm512_popcnt_epi32(a); }
EOF
cat >"$tmp/avx512_vnni.c" <<'EOF'
#include <immintrin.h>
__m512i dot(__m512i a, __m512i b, __m512i c) { return _mm512_dpbusd_epi32(c, a, b); }
EOF
cat >"$tmp/avx512_fma.c" <<'EOF'
#include <immintrin.h>
__m512 fma512f(__m512 a, __m512 b, __m512 c) { return _mm512_fmadd_ps(a, b, c); }
__m512d fma512d(__m512d a, __m512d b, __m512d c) { return _mm512_fmadd_pd(a, b, c); }
EOF
cat >"$tmp/avx512_fma_runtime.c" <<'EOF'
#include <immintrin.h>
__attribute__((noinline)) static float f32(float a, float b, float c) {
    __m512 x = _mm512_set1_ps(a), y = _mm512_set1_ps(b), z = _mm512_set1_ps(c);
    return _mm512_fmadd_ps(x, y, z)[0];
}
__attribute__((noinline)) static double f64(double a, double b, double c) {
    __m512d x = _mm512_set1_pd(a), y = _mm512_set1_pd(b), z = _mm512_set1_pd(c);
    return _mm512_fmadd_pd(x, y, z)[0];
}
int main(void) {
    if (!__builtin_cpu_supports("avx512f")) return 77;
    volatile float a = 0x1.000002p0f, b = 0x1.fffffcp-1f, c = -1.0f;
    volatile double x = 0x1.0000000000001p0, y = 0x1.ffffffffffffep-1, z = -1.0;
    return f32(a, b, c) == -0x1p-46f && f64(x, y, z) == -0x1p-104 ? 0 : 1;
}
EOF
reject avx512_bw_vl.c -mavx512bw -mno-avx512vl -march=x86-64
reject avx512_bw_vl.c -mavx512vl -mno-avx512bw -march=x86-64
accept avx512_bw_vl.c vpcmpub -mavx512bw -mavx512vl
accept avx512_bw_vl.c vpcmpub -mavx512bw -mavx512vl -mno-avx512bw -mavx512bw
reject avx512_bw_vl.c 'not implemented for i686' -m32 -mavx512bw -mavx512vl
reject avx512_ternlog_vl.c -mavx512f -mno-avx512vl -march=x86-64
accept avx512_ternlog_vl.c vpternlogd -mavx512f -mavx512vl
reject avx512_maskload_bw_vl.c -mavx512bw -mno-avx512vl -march=x86-64
accept avx512_maskload_bw_vl.c vmovdqu8 -mavx512bw -mavx512vl
reject avx512_f_qword.c -mavx512f -march=x86-64
accept avx512_f_qword.c vpabsq -march=x86-64 -mavx512f -mno-avx512dq
reject avx512_bitalg.c -mavx512f -mno-avx512bitalg
accept avx512_bitalg.c vpopcntb -mavx512bitalg
reject avx512_vpopcnt.c -mavx512f -mno-avx512vpopcntdq
accept avx512_vpopcnt.c vpopcntd -mavx512vpopcntdq
reject avx512_vnni.c -mavx512f -mno-avx512vnni
accept avx512_vnni.c vpdpbusd -mavx512vnni
# EVEX FMA belongs to AVX512F; VEX FMA remains FMA3 and is not conflated.
reject avx512_fma.c -mavx512f -march=x86-64
accept avx512_fma.c vfmadd -mavx512f

# Function-local FMA permission is independent of -ffp-contract; local AVX-512
# subset grants must work without leaking into a neighboring baseline function.
cat >"$tmp/local_fma.c" <<'EOF'
#include <immintrin.h>
__attribute__((target("fma"))) __m256 f(__m256 a, __m256 b, __m256 c) { return _mm256_fmadd_ps(a, b, c); }
EOF
accept local_fma.c vfmadd -march=x86-64
cat >"$tmp/local_bw_vl.c" <<'EOF'
#include <immintrin.h>
__attribute__((target("avx512bw,avx512vl"))) __mmask16 f(__m128i a, __m128i b) { return _mm_cmp_epu8_mask(a,b,_MM_CMPINT_EQ); }
EOF
accept local_bw_vl.c vpcmpub -march=x86-64
cat >"$tmp/local_bw_denied.c" <<'EOF'
#include <immintrin.h>
__attribute__((target("no-avx512bw"))) __mmask16 f(__m128i a, __m128i b) { return _mm_cmp_epu8_mask(a,b,_MM_CMPINT_EQ); }
EOF
reject local_bw_denied.c -mavx512bw -march=x86-64 -mavx512bw -mavx512vl
cat >"$tmp/local_fma_no_leak.c" <<'EOF'
#include <immintrin.h>
__attribute__((target("fma"))) __m256 f(__m256 a, __m256 b, __m256 c) { return _mm256_fmadd_ps(a,b,c); }
__m256 g(__m256 a, __m256 b, __m256 c) { return _mm256_fmadd_ps(a,b,c); }
EOF
reject local_fma_no_leak.c -mfma -march=x86-64

# GCC 16.2 is the authoritative verdict for feature flags, including each
# independent subset and its VL conjunction. Older system GCC is not used as
# an AVX-512 semantic oracle; run the exact compiler when the script is given.
if "$gcc_bin" -dumpfullversion 2>/dev/null | grep -x '16.2.0' >/dev/null; then
    for src in avx512_bw_vl.c avx512_ternlog_vl.c avx512_maskload_bw_vl.c avx512_f_qword.c avx512_bitalg.c avx512_vpopcnt.c avx512_vnni.c avx512_fma.c; do
        compare "$src" -march=x86-64
        compare "$src" -march=x86-64 -mavx512f -mavx512bw -mavx512dq -mavx512vl -mavx512bitalg -mavx512vpopcntdq -mavx512vnni
    done
    compare avx512_bw_vl.c -march=x86-64 -mavx512f -mavx512vl
    compare avx512_bw_vl.c -march=x86-64 -mavx512f -mavx512bw
    compare avx512_bw_vl.c -march=x86-64 -mavx512bw -mavx512vl
    compare avx512_f_qword.c -march=x86-64 -mavx512f -mno-avx512dq
    compare avx512_vnni.c -march=x86-64 -mavx512f -mno-avx512vnni
    compare local_fma.c -march=x86-64
    compare local_fma_no_leak.c -march=x86-64
    compare local_bw_vl.c -march=x86-64
    compare local_bw_denied.c -march=x86-64 -mavx512bw -mavx512vl

    "$ccc" -O2 -mavx512f -ffp-contract=off "$tmp/avx512_fma_runtime.c" -o "$tmp/lccc-avx512-fma"
    "$gcc_bin" -O2 -mavx512f -ffp-contract=off "$tmp/avx512_fma_runtime.c" -o "$tmp/gcc-avx512-fma"
    set +e
    "$tmp/lccc-avx512-fma"
    lccc_rc=$?
    set -e
    if [ "$lccc_rc" -eq 77 ]; then
        echo "SKIP: host CPU does not advertise AVX512F; FMA runtime fixture compiled with both compilers"
    else
        set +e
        "$tmp/gcc-avx512-fma"
        gcc_rc=$?
        set -e
        [ "$lccc_rc" -eq 0 ] && [ "$gcc_rc" -eq 0 ] ||
            fail "AVX512F fused multiply-add exact-rounding runtime: lccc=$lccc_rc gcc=$gcc_rc"
    fi
fi

echo "PASS: ISA-sensitive intrinsics are gated on the permitted feature set, oracle-checked"

# Legacy SSE1 / SSE2 permission, exact instruction ceiling and native ABI legs.
python3 "$repo/tests/regression/check_legacy_sse_permissions.py" --lccc "$ccc"
python3 "$repo/tests/regression/check_legacy_sse_native.py" --lccc "$ccc"
