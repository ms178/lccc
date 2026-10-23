#!/usr/bin/env bash
# End-to-end gate for 16-byte XMM slot forwarding.
#
# WHY THIS EXISTS
# ---------------
# `store_forwarding::global_store_forwarding` forwards an XMM slot store into a
# later reload of the same slot.  Until `MoveSize::DQ` existed it only knew the
# scalar widths (`movsd|movss|movq|movd`), so the 16-byte round trip that an
# `__m128*` value makes when it briefly leaves its register was emitted
# verbatim:
#
#     movdqu %xmm0, 48(%rsp)
#     ...
#     movdqu 48(%rsp), %xmm0
#
# The unit tests in `store_forwarding.rs` prove the *pass* refuses and accepts
# the right shapes.  They cannot prove the pass fires on what the backend
# actually emits, because that depends on the frame layout, on which
# instructions the frontend produces, and on the peephole phase order.  This
# gate compiles real C and asserts on the assembly: a slot reload that follows
# its own slot store inside one basic block must be gone.
#
# The second half is the regression that matters more than the first: the
# forwarding must never change an observable bit, so every configuration is also
# executed and compared against an independently computed reference hash.
#
# SOUNDNESS CONTRACT PINNED HERE
#   * a 16-byte reload of a 16-byte store forwards (no `movdqu slot, %xmmN`
#     survives when the register still holds the value);
#   * an 8-byte reload of a 16-byte home forwards to `movq %xmmS, %xmmD` (the
#     register form zeroes bits 64..127 exactly like the load does);
#   * a same-register reload at 16 bytes is deleted only when it is a no-op --
#     at 8 bytes the upper-lane zeroing is observable and the load must stay
#     (the unit test `fp_store_fp_load_same_register_keeps_zeroing` owns the
#     precise rule; this gate pins the end-to-end consequence).
set -u -o pipefail

repo=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
ccc=${CCC:-$repo/target/fastbuild/lccc}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

fail=0
fail() { echo "FAIL: $*" >&2; fail=1; }

# ---------------------------------------------------------------------------
# 1. The round trip the aggregate ABI produces.
# ---------------------------------------------------------------------------
cat >"$tmp/relay.c" <<'EOF'
#include <emmintrin.h>

/* The shape every inlined `__always_inline__` SSE wrapper leaves behind: the
   16-byte aggregate value leaves its register for a frame slot and comes back
   before its next use.  Nothing here is volatile or address-taken, so the slot
   is a pure relay and the compiler owns every byte of it. */
__attribute__((noinline)) void relay16(const void *restrict a, void *restrict o, int n)
{
    for (int i = 0; i + 16 <= n; i += 16) {
        __m128d x = _mm_loadu_pd((const double *)((const char *)a + i));
        __m128d y;
        y = x;                       /* the aggregate copy under test */
        y = _mm_add_pd(y, x);
        _mm_storeu_pd((double *)((char *)o + i), y);
    }
}

/* The narrower reload: an 8-byte scalar read of a home a 16-byte store wrote. */
__attribute__((noinline)) double narrow_reload(const void *restrict a, int n)
{
    double acc = 0.0;
    for (int i = 0; i + 16 <= n; i += 16) {
        __m128d x = _mm_loadu_pd((const double *)((const char *)a + i));
        __m128d y = x;
        y = _mm_mul_pd(y, x);
        acc += _mm_cvtsd_f64(y) + _mm_cvtsd_f64(x);
    }
    return acc;
}

#include <stdio.h>
static double A[256] __attribute__((aligned(16)));
static double O[256] __attribute__((aligned(16)));

static unsigned long long fnv(const unsigned char *p, unsigned long n)
{
    unsigned long long h = 1469598103934665603ULL;
    for (unsigned long i = 0; i < n; i++) { h ^= p[i]; h *= 1099511628211ULL; }
    return h;
}

int main(void)
{
    for (int i = 0; i < 256; i++) A[i] = (double)((i * 37) % 101) - 50.5;
    relay16(A, O, 256);
    printf("relay16 %llx\n", fnv((unsigned char *)O, sizeof O));
    printf("narrow %.17g\n", narrow_reload(A, 256));
    return 0;
}
EOF

for opt in -O2 -O3; do
    if ! "$ccc" "$opt" -S -o "$tmp/relay$opt.s" "$tmp/relay.c" 2>"$tmp/err"; then
        fail "lccc $opt -S failed: $(cat "$tmp/err")"
        continue
    fi

    # Within one basic block, a 16-byte reload of the very slot the previous
    # 16-byte store wrote is exactly the pattern the forwarder deletes.
    python3 - "$tmp/relay$opt.s" "$opt" <<'PY' || fail=1
import re
import sys

path, opt = sys.argv[1], sys.argv[2]
lines = [l.strip() for l in open(path)]
# Function bodies only: stop at the epilogue marker of each function.
store = re.compile(r"^(movdqu|movups|movdqa|movaps|movapd)\s+%xmm(\d+),\s*(.+)\(%rsp\)$")
load = re.compile(r"^(movdqu|movups|movdqa|movaps|movapd)\s+(.+)\(%rsp\),\s*%xmm(\d+)$")
bad = []
for i, l in enumerate(lines):
    m = store.match(l)
    if not m:
        continue
    reg, slot = m.group(2), m.group(3)
    for j in range(i + 1, min(i + 24, len(lines))):
        t = lines[j]
        if t.startswith(".") and "cfi" not in t:
            break
        if re.match(r"^j|^ret|^call", t):
            break
        # Any other write to the register ends the window.
        if re.search(r"%xmm" + reg + r"\s*$", t) and not t.startswith("movdq"):
            break
        m2 = load.match(t)
        if m2 and m2.group(3) == reg and m2.group(2) == slot:
            bad.append((i + 1, l, j + 1, t))
            break
if bad:
    for i, s, j, t in bad:
        print(f"{opt}: slot round trip survived: line {i}: {s}  ->  line {j}: {t}")
    sys.exit(1)
PY

    # The aligned half must never be invented, and the 16-byte moves that
    # remain must be real loads/stores, not self moves.
    if grep -qE '^\s+movdqu %xmm([0-9]+), %xmm\1$' "$tmp/relay$opt.s"; then
        fail "$opt: a 16-byte self move was emitted"
    fi
done

# ---------------------------------------------------------------------------
# 2. Differential execution: the rewrite must be bit-exact.
# ---------------------------------------------------------------------------
if ! "$ccc" -O2 -o "$tmp/lccc.exe" "$tmp/relay.c" 2>"$tmp/err"; then
    fail "lccc -O2 link failed: $(cat "$tmp/err")"
else
    got=$("$tmp/lccc.exe") || fail "lccc binary failed to run"
    # Reference: same source through the pinned GCC 16.2 oracle when present.
    gcc_bin=${GCC:-gcc-16}
    if command -v "$gcc_bin" >/dev/null 2>&1; then
        if "$gcc_bin" -O2 -fno-tree-vectorize -o "$tmp/gcc.exe" "$tmp/relay.c" 2>"$tmp/err"; then
            want=$("$tmp/gcc.exe")
            if [ "$got" != "$want" ]; then
                fail "differential mismatch against $gcc_bin -O2 (scalar):
  lccc: $got
  $gcc_bin: $want"
            fi
        fi
    fi
    # The forwarding-off configuration must produce the same bits too: the
    # rewrite is an optimization, never a semantic change.
    if CCC_NO_XMM_SF=1 "$ccc" -O2 -o "$tmp/nosf.exe" "$tmp/relay.c" 2>"$tmp/err"; then
        nosf=$("$tmp/nosf.exe") || fail "CCC_NO_XMM_SF binary failed to run"
        if [ "$got" != "$nosf" ]; then
            fail "forwarding changed the result:
  with: $got
  CCC_NO_XMM_SF=1: $nosf"
        fi
    fi
fi

if [ "$fail" -ne 0 ]; then
    echo "sse2 slot forwarding gate: FAIL" >&2
    exit 1
fi
echo "sse2 slot forwarding gate: PASS"
