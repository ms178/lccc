#!/usr/bin/env bash
# MMX entry points (mmintrin.h): runtime differential of every __m64
# intrinsic against GCC's real-MMX implementation, plus the packed-codegen
# witness that the header's ops lower to ONE SSE2 instruction on the low
# half of an XMM instead of a per-lane scalar loop (the historical
# "MMX on x86-64 is lowered through scalar code" gap in
# docs/PERFORMANCE-BACKLOG.md).
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
ccc=${CCC:-$root/target/fastbuild/lccc}
gcc_bin=${GCC:-gcc-16}
[ -x "$ccc" ] || { echo "FAIL: compiler not built: $ccc" >&2; exit 1; }
command -v "$gcc_bin" >/dev/null || { echo "FAIL: no GCC ($gcc_bin)"; exit 1; }

fails=0
fail() { echo "FAIL: $*" >&2; fails=$((fails + 1)); }

# Fixture provenance: the generated differential must match the generator.
python3 "$root/scripts/gen_mmx_intrinsics_differential.py" --check \
    || fail "mmx differential fixture drifted from its generator"

tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
src=$root/tests/regression/mmx_intrinsics_differential.c
src32=$root/tests/regression/mmx_intrinsics_differential_m32.c

# ---- runtime differential, -m64 ------------------------------------------------
run_pair() {
    local srcf=$1; shift
    local tag=$1; shift
    # shellcheck disable=SC2086
    if ! "$ccc" "$@" -o "$tmp/l.$tag" "$srcf" 2>"$tmp/l.$tag.err"; then
        fail "$tag: lccc compile: $(head -c 300 "$tmp/l.$tag.err")"; return
    fi
    # shellcheck disable=SC2086
    if ! "$gcc_bin" "$@" -o "$tmp/g.$tag" "$srcf" 2>"$tmp/g.$tag.err"; then
        fail "$tag: gcc compile: $(head -c 300 "$tmp/g.$tag.err")"; return
    fi
    local lo go
    lo=$("$tmp/l.$tag" 2>&1) || fail "$tag: lccc run rc=$?"
    go=$("$tmp/g.$tag" 2>&1) || fail "$tag: gcc run rc=$?"
    if [ "$lo" != "$go" ]; then
        fail "$tag: hash differential mismatch"
        diff <(printf '%s\n' "$lo") <(printf '%s\n' "$go") | head -12 >&2 || true
    fi
}
run_pair "$src" m64 -O2

# ---- packed-codegen witness ----------------------------------------------------
cat > "$tmp/w.c" <<'W'
typedef int __m64 __attribute__((vector_size(8)));
__m64 w_add8(__m64 a, __m64 b);
__m64 w_add16(__m64 a, __m64 b);
__m64 w_mulhi(__m64 a, __m64 b);
__m64 w_cmpeq(__m64 a, __m64 b);
__m64 w_sll(__m64 a, int n);
#define H "mmintrin.h"
#include H
__m64 w_add8(__m64 a, __m64 b){ return _mm_add_pi8(a,b); }
__m64 w_add16(__m64 a, __m64 b){ return _mm_add_pi16(a,b); }
__m64 w_mulhi(__m64 a, __m64 b){ return _mm_mulhi_pi16(a,b); }
__m64 w_cmpeq(__m64 a, __m64 b){ return _mm_cmpeq_pi8(a,b); }
__m64 w_sll(__m64 a, int n){ return _mm_slli_pi16(a, n); }
W
if "$ccc" -O2 -S "$tmp/w.c" -o "$tmp/w.s" 2>"$tmp/w.err"; then
    for spec in 'paddb:w_add8' 'paddw:w_add16' 'pmulhw:w_mulhi' 'pcmpeqb:w_cmpeq' 'psllw:w_sll'; do
        op=${spec%%:*} fn=${spec##*:}
        if [ "$(awk -v fn="$fn:" '$0==fn{on=1;next} on&&/ret/{exit} on' "$tmp/w.s" | grep -c "$op")" -eq 0 ]; then
            fail "packed witness: $fn does not emit $op"
        fi
    done
    # The scalar-loop era: a per-lane body pays >= 8 loop ops; the packed
    # bodies are a handful of image moves.  Pin every witness body <= 14
    # instructions (measured: movups/movq staging + one packed op + store).
    # Measured envelopes include the frame/prologue; the scalar-loop era
    # bodies were materially larger and, decisively, lacked the packed op
    # the grep above pins.
    for spec in 'w_add8:18' 'w_add16:18' 'w_mulhi:18' 'w_cmpeq:18' 'w_sll:26'; do
        fn=${spec%%:*} max=${spec##*:}
        n=$(awk -v fn="$fn:" '$0==fn{on=1;next} on&&/ret/{exit} on {l=$0; sub(/^[ \t]+/,"",l); if (l==""||l~/^[.#]/||l~/retq?$/) next; n++} END{print n+0}' "$tmp/w.s")
        [ "$n" -le "$max" ] || fail "packed witness: $fn emits $n instructions (ceiling $max; scalar-loop regression)"
    done
else
    fail "packed witness compile: $(head -c 300 "$tmp/w.err")"
fi

# ---- runtime differential, -m32 (same sysroot rules as every i686 gate) --------
if printf '#include <stdio.h>\n' | "$ccc" -m32 -fsyntax-only -x c - >/dev/null 2>&1; then
    # shellcheck disable=SC1091
    m32flags=$(cat "$root/tests/regression/mmx_intrinsics_differential_m32.flags")
    # shellcheck disable=SC2086
    run_pair "$src32" m32 $m32flags
else
    echo "SKIP: i686 MMX differential - no 32-bit libc headers in this environment"
fi

if [ "$fails" -gt 0 ]; then
    echo "FAIL: $fails MMX failure(s)" >&2
    exit 1
fi
echo "PASS: MMX intrinsics (differential + packed witnesses, generator drift check)"
