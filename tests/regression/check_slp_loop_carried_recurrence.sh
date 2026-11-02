#!/usr/bin/env bash
# BB-SLP must not pack a store→load recurrence that crosses the loop backedge.
#
# `m[i] = f(m[i-2]) + m[i-1]` (SHA-256's message schedule, and every stencil
# with a distance-1 tap) reads exactly the bytes the previous iteration's
# store wrote. Packing the group replaces a register carry with a
# store→load round trip through L1D, and it pre-empts
# `loop_carried_forward` — that pass models scalar `Load`/`Store` only, so a
# `VecLoad*`/`VecStore*` in the body makes it fail the whole loop closed even
# though it would have built the header phi.
#
# Measured on sha256_transform: the packed form is 12 % slower than the
# scalar form of the same compiler and 11 % slower than the pipeline before
# the packing started firing. This gate pins all three properties:
#
#   1. the packed and scalar forms of the same compiler agree at run time;
#   2. the schedule loop performs no store→load round trip (the pack reload
#      is the only negative-displacement `movq` xmm load in the file);
#   3. SLP is still ALIVE — an ordinary `c[i] = a[i] + b[i]` map must still
#      emit a packed load/add/store, so the guard cannot silently degenerate
#      into "SLP does nothing".
set -euo pipefail
cd "$(dirname "$0")/../.."
ccc=${LCCC_BIN:-${CCC:-target/fastbuild/lccc}}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

# ── 1. packed and scalar forms must agree ────────────────────────────────
src=tests/benchmark/programs/sha256_transform.c
"$ccc" -O2 "$src" -o "$td/on"
CCC_NO_BB_SLP=1 "$ccc" -O2 "$src" -o "$td/off"
"$td/on" >"$td/on.out"
"$td/off" >"$td/off.out"
cmp -s "$td/on.out" "$td/off.out" || {
    echo "--- SLP on ---" >&2; cat "$td/on.out" >&2
    echo "--- SLP off ---" >&2; cat "$td/off.out" >&2
    fail "SLP-on and SLP-off disagree on sha256_transform"
}

# ── 2. no store→load round trip in the message schedule ──────────────────
# The schedule packs `m[i-2..i-1]` into one xmm pair and stores it; reading
# that pair back is a `movq -8(%rsi), %xmm` shape. The scalar form has no
# xmm memory operand at all in the loop, so any negative-displacement xmm
# load is the round trip this gate forbids.
"$ccc" -O2 -S "$src" -o "$td/on.s"
if grep -nE '^[[:space:]]+movq[[:space:]]+-[0-9]+\(%r[a-z0-9]+\),[[:space:]]*%xmm' "$td/on.s" >"$td/reloads"; then
    sed 's/^/  /' "$td/reloads" >&2
    fail "BB-SLP packed a loop-carried store->load recurrence (sha256_transform)"
fi

# ── 3. SLP must still vectorize an ordinary map ──────────────────────────
cat >"$td/map.c" <<'C'
#include <stdio.h>
#include <stdint.h>
#define N 128
static uint32_t a[N], b[N], c[N];
int main(void) {
    for (int i = 0; i < N; i++) {
        a[i] = (uint32_t)i * 2654435761u;
        b[i] = (uint32_t)i ^ 0x9e3779b9u;
    }
    for (int i = 0; i < N; i++)
        c[i] = a[i] + b[i];
    printf("%08x\n", c[N - 1]);
    return 0;
}
C
"$ccc" -O2 -S "$td/map.c" -o "$td/map.s"
grep -qE '^[[:space:]]+(v?paddd|v?paddq)' "$td/map.s" \
    || fail "SLP no longer packs an unpacked map loop (c[i] = a[i] + b[i])"
"$ccc" -O2 "$td/map.c" -o "$td/map"
"$td/map" >/dev/null

# ── 4. the half-wide dword-pair family must still be reachable ────────────
# d4's schedule used to be its only instantiation, and that shape is now
# declined (packing it measured 15.6 % slower). The family itself is not
# dead: a strided pair store fed by two contiguous pair loads has no
# recurrence — the load stream and the store stream are different arrays,
# so the guard's same-stream test never matches — and still packs.
cat >"$td/pair.c" <<'C'
#include <stdio.h>
#include <stdint.h>
#define N 2048
static uint32_t a[2 * N], b[2 * N], o[2 * N];
int main(void) {
    for (int i = 0; i < 2 * N; i++) {
        a[i] = (uint32_t)i * 2654435761u;
        b[i] = (uint32_t)i ^ 0x9e3779b9u;
    }
    for (int i = 0; i < 2 * N; i += 2) {
        o[i] = a[i] + b[i];
        o[i + 1] = a[i + 1] + b[i + 1];
    }
    printf("%08x\n", o[2 * N - 1] + o[7]);
    return 0;
}
C
"$ccc" -O2 -march=x86-64-v3 -S "$td/pair.c" -o "$td/pair.s"
pairs=$(grep -cE '^[[:space:]]+v?movq[[:space:]]+.*%xmm' "$td/pair.s" || true)
[ "$pairs" -ge 2 ] \
    || fail "the half-wide dword-pair family stopped firing (movq/vmovq pair ops=$pairs)"

# ── 5. a nearby same-stream load with a different stride is independent ───
# The pair store is two words ahead of its loads, but the loop advances four
# words, so it does not re-read the previous iteration's store. Declining it
# solely on the displacement costs vectorization. Check the result and require
# packed arithmetic in this function, not just somewhere else in the file.
"$ccc" -O2 tests/regression/slp_independent_gap.c -o "$td/gap"
[ "$("$td/gap")" = "slp independent gap: ok" ] || fail "independent-gap runtime"
"$ccc" -O2 -march=x86-64-v3 -S tests/regression/slp_independent_gap.c -o "$td/gap.s"
awk '/^gap_pairs:/{inside=1} inside{print} /[.]size[[:space:]]+gap_pairs/{inside=0}' "$td/gap.s" >"$td/gap-fn.s"
grep -qE '^[[:space:]]+v?(paddd|psrld)' "$td/gap-fn.s" \
    || fail "independent same-stream gap loop stopped vectorizing"

# ── 6. declining a seed must not corrupt the block's SSA order ────────────
# `pair_schedule_carry` runs the same schedule at nine lengths. When the
# guard declined one group, the surviving group's consumer landed before
# the producer another group had pinned, and the phase-boundary verifier
# rejected the block ("after iter=0 slp_late"). The plan is now verified
# before it is committed.
CCC_VALIDATE_SSA=1 "$ccc" -O2 tests/regression/pair_schedule_carry.c -o "$td/psc"
"$td/psc" >/dev/null

echo "ok: BB-SLP declines loop-carried recurrences and still packs plain maps"
