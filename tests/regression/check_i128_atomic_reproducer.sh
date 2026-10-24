#!/usr/bin/env bash
# TODO-7 / FOLLOWUP-2026-10-21-i128-atomic-o2.md: the i128-atomic consumer
# miscompile at -O2 (a middle-end pass reusing a stale value once the hashing
# consumer was inlined; wrong at -O2/-O3, correct at -O0/-O1, fixed
# 2026-10-21). This gate pins the minimal reproducer VERBATIM from the
# follow-up document so the class cannot silently return:
#   * lccc must agree with gcc-16.2 on the program's exit status (the low
#     32 bits of the accumulator hash) at every optimisation level, and
#   * under the two compare-exchange configs that split by lowering — the
#     default libatomic call form and -mcx16's inline cmpxchg16b — because
#     the original defect tracked the call form.
set -euo pipefail
repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
ccc=${CCC:-$repo/target/fastbuild/lccc}
gcc=${GCC16:-${GCC:-gcc-16}}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

if ! "$gcc" --version 2>/dev/null | grep -c "16\.2" >/dev/null; then
    echo "SKIP: gcc-16.2 oracle not on PATH"
    exit 0
fi

cat > "$tmp/repro.c" <<'C'
typedef unsigned __int128 u128;
static _Alignas(16) _Atomic u128 v;
static unsigned long long h;

int main(void) {
    u128 k = ((u128)0x0123456789ABCDEFULL << 64) | 0xFEDCBA9876543210ULL;
#define MIX(x) do { u128 t_ = (x); \
    h = h * 1000003ULL + (unsigned long long)t_ + (unsigned long long)(t_ >> 64); \
} while (0)
    {   __atomic_store_n(&v, k + 1, __ATOMIC_RELAXED);
        u128 e = k + 1;
        MIX((u128)__atomic_compare_exchange_n(&v, &e, k + 1 + (u128)1, 0,
                                              __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST));
        MIX(e); MIX(v); }
    {   __atomic_store_n(&v, k + 2, __ATOMIC_RELAXED);
        u128 e = k + 2;
        MIX((u128)__atomic_compare_exchange_n(&v, &e, k + 2 + (u128)1, 0,
                                              __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST));
        MIX(e); MIX(v); }
    return (int)h;   /* gcc 16.2 -O2: 0xd674e05bde401cdc, lccc -O2 (buggy): 0xb5afa0312156b616 */
}
C

fail=0
for cfg in "" "-mcx16"; do
    for opt in -O0 -O1 -O2 -O3; do
        "$gcc" $opt $cfg "$tmp/repro.c" -o "$tmp/ref" -latomic
        "$ccc" $opt $cfg "$tmp/repro.c" -o "$tmp/mine" -latomic
        # The exit status IS the assertion (the low bits of the accumulator
        # hash); a hashing program exits non-zero by design, so step out of
        # errexit around the runs.
        set +e
        "$tmp/ref"; ref=$?
        "$tmp/mine"; mine=$?
        set -e
        if [ "$ref" != "$mine" ]; then
            echo "DIVERGED: $opt ${cfg:-default}: gcc exit $ref, lccc exit $mine"
            fail=1
        fi
    done
done
if [ "$fail" != 0 ]; then
    exit 1
fi
echo "PASS: i128 atomic consumer reproducer agrees with gcc-16.2 at -O0..-O3, default and -mcx16"
