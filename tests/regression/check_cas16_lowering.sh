#!/usr/bin/env bash
# 16-byte compare-exchange lowering (`__atomic_compare_exchange_n` and
# `__sync_val_compare_and_swap` on unsigned __int128).
#
# Contract:
#   * default build (no -mcx16, plain x86-64): a libatomic call to
#     __atomic_compare_exchange_16, exactly as GCC lowers it; no cmpxchg16b.
#   * -mcx16 (or a v2+ -march profile): one inline `lock cmpxchg16b`.
#   * both compare and write the FULL 128 bits: a mismatch that lives only in
#     the high half must fail, and a success must store both halves.  The old
#     lowering used a 64-bit `lock cmpxchg` and got this wrong.
# The runtime checks are cross-checked against GCC 16 for the same program.
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
ccc=${CCC:-$repo/target/fastbuild/lccc}
gcc=${GCC16:-gcc-16}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

[ -x "$ccc" ] || { echo "SKIP: $ccc not built" >&2; exit 0; }
command -v objdump >/dev/null || { echo "SKIP: objdump missing" >&2; exit 0; }
fail() { echo "FAIL: $*" >&2; exit 1; }

cat >"$tmp/cas.c" <<'EOF'
typedef unsigned __int128 u128;
unsigned long long cas_bool(u128 *p, u128 e, u128 d) {
    return __atomic_compare_exchange_n(p, &e, d, 0, __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST);
}
u128 cas_val(u128 *p, u128 e, u128 d) {
    return __sync_val_compare_and_swap(p, e, d);
}
EOF

cat >"$tmp/run.c" <<'EOF'
#include <stdio.h>
typedef unsigned __int128 u128;
static int bad;
#define CHECK(c) do { if (!(c)) { printf("FAIL line %d: %s\n", __LINE__, #c); bad = 1; } } while (0)
int main(void) {
    u128 d = ((u128)0x3333u << 64) | 0x4444u;
    u128 v = ((u128)0x1111u << 64) | 0x2222u;
    u128 e = v;
    _Bool r = __atomic_compare_exchange_n(&v, &e, d, 0, __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST);
    CHECK(r);
    CHECK(v == d);
    /* Mismatch only in the high half: must fail, must not write, and must
       write the observed value back into the expected operand. */
    u128 e2 = (u128)0x4444u;
    r = __atomic_compare_exchange_n(&v, &e2, (u128)7, 0, __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST);
    CHECK(!r);
    CHECK(v == d);
    CHECK(e2 == d);
    /* Success that changes only the high half: both halves must be stored. */
    u128 w = ((u128)0x9999u << 64) | 0x4444u;
    e = d;
    r = __atomic_compare_exchange_n(&v, &e, w, 0, __ATOMIC_ACQ_REL, __ATOMIC_ACQUIRE);
    CHECK(r);
    CHECK(v == w);
    /* Value-returning form: the observed old value comes back on mismatch. */
    u128 old = __sync_val_compare_and_swap(&v, (u128)0x4444u, (u128)1);
    CHECK(old == w);
    CHECK(v == w);
    old = __sync_val_compare_and_swap(&v, w, (u128)1);
    CHECK(old == w);
    CHECK(v == 1);
    printf(bad ? "FAILED\n" : "PASS\n");
    return bad;
}
EOF

# 1. Default lowering: libatomic call, no inline cmpxchg16b.
"$ccc" -O2 -c "$tmp/cas.c" -o "$tmp/cas_def.o" || fail "lccc default build of cas.c"
dis=$(objdump -dr "$tmp/cas_def.o")
grep -q "__atomic_compare_exchange_16" <<<"$dis" \
    || fail "default build does not call __atomic_compare_exchange_16"
! grep -q "cmpxchg16b" <<<"$dis" || fail "default build emitted cmpxchg16b (post-baseline)"
echo "PASS: default 16-byte CAS calls libatomic (no cmpxchg16b)"

# 2. -mcx16: inline cmpxchg16b, no libatomic call.
"$ccc" -O2 -mcx16 -c "$tmp/cas.c" -o "$tmp/cas_cx.o" || fail "lccc -mcx16 build of cas.c"
dis=$(objdump -dr "$tmp/cas_cx.o")
grep -q "lock cmpxchg16b" <<<"$dis" || fail "-mcx16 build lacks lock cmpxchg16b"
! grep -q "__atomic_compare_exchange_16" <<<"$dis" \
    || fail "-mcx16 build still calls libatomic for a 16-byte CAS"
echo "PASS: -mcx16 16-byte CAS is an inline lock cmpxchg16b"

# 2b. The permission is sticky and order-independent: an explicit -mno-cx16
#     beats a later v2 profile or a v3 grant; CMPXCHG16B is integer ISA and
#     survives -mno-sse.
for flags in "-march=x86-64-v3 -mno-cx16" "-mno-cx16 -march=x86-64-v2"; do
    "$ccc" -O2 $flags -c "$tmp/cas.c" -o "$tmp/cas_deny.o" || fail "lccc $flags build"
    dis=$(objdump -dr "$tmp/cas_deny.o")
    ! grep -q "cmpxchg16b" <<<"$dis" || fail "'$flags' emitted cmpxchg16b despite -mno-cx16"
    grep -q "__atomic_compare_exchange_16" <<<"$dis" || fail "'$flags' lacks the libatomic call"
done
echo "PASS: -mno-cx16 is sticky against v2/v3 grants"
"$ccc" -O2 -mno-sse -mcx16 -c "$tmp/cas.c" -o "$tmp/cas_nosse.o" || fail "lccc -mno-sse -mcx16 build"
nosse=$(objdump -dr "$tmp/cas_nosse.o")
grep -q "lock cmpxchg16b" <<<"$nosse" \
    || fail "-mno-sse -mcx16 lost the inline cmpxchg16b (integer ISA)"
echo "PASS: CMPXCHG16B survives -mno-sse"

# 3. Runtime semantics. The GCC 16 oracle needs -mcx16: without it GCC cannot
#    link the __sync 16-byte form at all, so it is only usable with the flag.
#    Everything runtime-related needs a host with CMPXCHG16B.
if ! grep -qw cx16 /proc/cpuinfo 2>/dev/null; then
    echo "SKIP: host lacks cx16; runtime 16-byte CAS checks not exercised" >&2
    echo "PASS: 16-byte CAS lowering (static checks only)"
    exit 0
fi
command -v "$gcc" >/dev/null || { echo "SKIP: $gcc missing; runtime oracle not run" >&2; echo "PASS: 16-byte CAS lowering (static checks only)"; exit 0; }
"$gcc" -O2 -mcx16 "$tmp/run.c" -o "$tmp/run_gcc" -latomic || fail "GCC 16 oracle did not build"
expect=$("$tmp/run_gcc") || true
[ "$expect" = "PASS" ] || fail "GCC 16 oracle does not agree: $expect"

"$ccc" -O2 "$tmp/run.c" -o "$tmp/run_def" -latomic || fail "lccc default build of run.c"
got=$("$tmp/run_def") || true
[ "$got" = "PASS" ] || fail "lccc default runtime: $got"
echo "PASS: default-build 128-bit CAS runtime matches GCC 16"

"$ccc" -O2 -mcx16 "$tmp/run.c" -o "$tmp/run_cx" -latomic || fail "lccc -mcx16 build of run.c"
got=$("$tmp/run_cx") || true
[ "$got" = "PASS" ] || fail "lccc -mcx16 runtime: $got"
echo "PASS: -mcx16 128-bit CAS runtime matches GCC 16"
echo "PASS: 16-byte CAS lowering (default = libatomic, -mcx16 = cmpxchg16b, full 128-bit compare)"
