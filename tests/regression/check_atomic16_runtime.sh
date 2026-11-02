#!/usr/bin/env bash
# 16-byte atomic load / store / read-modify-write on unsigned __int128.
#
# Contract, all of it cross-checked against GCC 16 for the same program:
#   * Every 16-byte atomic load, store and RMW calls libatomic's sized entry
#     (__atomic_load_16 / __atomic_store_16 / __atomic_fetch_*_16 /
#     __atomic_exchange_16), exactly as GCC lowers them at -O2 with and
#     without -mcx16/-mavx2.  x86-64 has no 16-byte XCHG/XADD and no
#     lock-prefixed 16-byte ALU form, so there is no inline spelling.
#   * The FULL 128 bits are transferred.  The 64-bit `movq`/`xchgq` forms
#     would silently drop the high half, which is the failure this gate
#     exists to catch: it corrupts the value AND is non-atomic for the pair.
#   * Semantics hold for every ordering and for both the -mcx16 (inline
#     cmpxchg16b) and default (libatomic) compare-exchange paths.
#
#   * No caller-saved register home spans a 16-byte atomic: every libatomic
#     entry is a real call (and the inline cmpxchg16b sequence stages through
#     the stack), so liveness models them as call points.  Before that, -O2
#     homed values in %r10/%r11 across `call __atomic_*_16@PLT` and the
#     peephole deleted their definitions (wrong hashes at -O2 only).
# Exercise the optimized arithmetic consumers too: a sized libatomic call can
# be correct while later I128 arithmetic or call-clobber handling is not.
# The independent check_i128_atomic_call_clobber.sh gate covers the live-value
# reproducer at -O0 through -O3 and across five ISA configurations.
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
ccc=${CCC:-$repo/target/fastbuild/lccc}
gcc=${GCC16:-gcc-16}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

[ -x "$ccc" ] || { echo "SKIP: $ccc not built" >&2; exit 0; }
command -v objdump >/dev/null || { echo "SKIP: objdump missing" >&2; exit 0; }
fail() { echo "FAIL: $*" >&2; exit 1; }

# ---------------------------------------------------------------- lowering ---
cat >"$tmp/lower.c" <<'EOF'
typedef unsigned __int128 u128;
void st16(_Atomic u128 *p, u128 a) { __atomic_store_n(p, a, __ATOMIC_SEQ_CST); }
void strl(_Atomic u128 *p, u128 a) { __atomic_store_n(p, a, __ATOMIC_RELAXED); }
u128 ld16(_Atomic u128 *p) { return __atomic_load_n(p, __ATOMIC_SEQ_CST); }
u128 fadd(_Atomic u128 *p, u128 a) { return __atomic_fetch_add(p, a, __ATOMIC_SEQ_CST); }
u128 fsub(_Atomic u128 *p, u128 a) { return __atomic_fetch_sub(p, a, __ATOMIC_SEQ_CST); }
u128 fand(_Atomic u128 *p, u128 a) { return __atomic_fetch_and(p, a, __ATOMIC_SEQ_CST); }
u128 foor(_Atomic u128 *p, u128 a) { return __atomic_fetch_or(p, a, __ATOMIC_SEQ_CST); }
u128 fxor(_Atomic u128 *p, u128 a) { return __atomic_fetch_xor(p, a, __ATOMIC_SEQ_CST); }
u128 fnnd(_Atomic u128 *p, u128 a) { return __atomic_fetch_nand(p, a, __ATOMIC_SEQ_CST); }
u128 fxch(_Atomic u128 *p, u128 a) { return __atomic_exchange_n(p, a, __ATOMIC_SEQ_CST); }
EOF

for opt in -O1 -O2 -O3; do
    for flags in "" "-mcx16"; do
        lbl="$opt ${flags:-default}"
        "$ccc" "$opt" $flags -c "$tmp/lower.c" -o "$tmp/lower.o" \
            || fail "lccc $lbl build of lower.c"
        dis=$(objdump -dr "$tmp/lower.o")
        # Each sized entry point must be called...
        for sym in __atomic_store_16 __atomic_load_16 __atomic_fetch_add_16 \
            __atomic_fetch_sub_16 __atomic_fetch_and_16 __atomic_fetch_or_16 \
            __atomic_fetch_xor_16 __atomic_fetch_nand_16 __atomic_exchange_16; do
            grep -q "$sym" <<<"$dis" || fail "$lbl lowering does not call $sym"
        done
        # ...and no 16-byte atomic may be lowered to a 64-bit locked op, which is
        # how the truncation bug presented itself.
        if grep -qE '\b(lock )?(xchgq|xaddq)\b' <<<"$dis"; then
            fail "$lbl lowering uses a 64-bit xchg/xadd for a 16-byte atomic"
        fi
    done
done
echo "PASS: 16-byte load/store/RMW lower to libatomic's sized entries"

# ----------------------------------------------------------------- runtime ---
cat >"$tmp/run.c" <<'EOF'
#include <stdio.h>
typedef unsigned __int128 u128;
static _Alignas(16) _Atomic u128 v;
static int bad;
#define CHECK(c) do { if (!(c)) { printf("FAIL line %d: %s\n", __LINE__, #c); bad = 1; } } while (0)
#define HI(x) ((unsigned long long)((x) >> 64))
#define LO(x) ((unsigned long long)(x))

int main(void) {
    /* Distinct high and low halves everywhere: a lowering that drops either
       half, or that swaps them, fails. */
    const u128 a = ((u128)0x0123456789ABCDEFULL << 64) | 0xFEDCBA9876543210ULL;

    /* store + load must round-trip both halves, for every ordering. */
    __atomic_store_n(&v, a, __ATOMIC_RELAXED);
    CHECK(__atomic_load_n(&v, __ATOMIC_RELAXED) == a);
    __atomic_store_n(&v, a, __ATOMIC_RELEASE);
    CHECK(__atomic_load_n(&v, __ATOMIC_ACQUIRE) == a);
    __atomic_store_n(&v, a, __ATOMIC_SEQ_CST);
    CHECK(__atomic_load_n(&v, __ATOMIC_SEQ_CST) == a);

    /* Every RMW must return the previous value and leave the new one, with
       the high half participating in the arithmetic. */
    u128 old;
    old = __atomic_fetch_add(&v, (u128)1, __ATOMIC_SEQ_CST);
    CHECK(old == a); CHECK(v == a + 1);
    old = __atomic_fetch_sub(&v, (u128)1, __ATOMIC_SEQ_CST);
    CHECK(old == a + 1); CHECK(v == a);
    old = __atomic_fetch_and(&v, a & ~((u128)0xFF << 64), __ATOMIC_SEQ_CST);
    CHECK(old == a); CHECK(v == (a & ~((u128)0xFF << 64)));
    old = __atomic_fetch_or(&v, (u128)0xFF << 64, __ATOMIC_SEQ_CST);
    CHECK(v == (a | ((u128)0xFF << 64)));
    __atomic_store_n(&v, a, __ATOMIC_SEQ_CST);
    old = __atomic_fetch_xor(&v, a, __ATOMIC_SEQ_CST);
    CHECK(old == a); CHECK(v == 0);
    __atomic_store_n(&v, a, __ATOMIC_SEQ_CST);
    old = __atomic_fetch_nand(&v, a, __ATOMIC_SEQ_CST);
    CHECK(old == a); CHECK(v == ~a);
    old = __atomic_exchange_n(&v, a, __ATOMIC_SEQ_CST);
    CHECK(old == ~a); CHECK(v == a);
    (void)old;

    /* The high half must participate in the compare: a difference confined to
       bits 64..127 must fail and write the observed value back. */
    u128 near = a ^ ((u128)1 << 127);
    u128 e = near;
    CHECK(!__atomic_compare_exchange_n(&v, &e, (u128)7, 0,
                                       __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST));
    CHECK(e == a);
    CHECK(v == a);

    /* Success path, and the low half must participate too. */
    e = a;
    CHECK(__atomic_compare_exchange_n(&v, &e, (u128)9, 0,
                                      __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST));
    CHECK(v == (u128)9);
    CHECK(e == a);

    printf("lccc: hi=%016llx lo=%016llx bad=%d\n", HI(v), LO(v), bad);
    return bad;
}
EOF

for opt in -O1 -O2 -O3; do
    for flags in "" "-mcx16" "-march=x86-64-v3"; do
        lbl="$opt ${flags:-default}"
        "$ccc" "$opt" $flags "$tmp/run.c" -o "$tmp/run_lccc" -latomic \
            || fail "lccc $lbl build of run.c"
        out=$("$tmp/run_lccc") || fail "lccc $lbl runtime: $out"
        grep -q 'bad=0' <<<"$out" || fail "lccc $lbl 16-byte atomic semantics: $out"
        echo "PASS: 16-byte atomics are correct at runtime ($lbl): $out"
    done
done

# ------------------------------------------------- caller-saved clobbers -----
# Values live across each kind of 16-byte atomic, at -O2 where the allocator
# hands out %r10/%r11 and xmm homes.  Expected output measured with gcc 16.2.
cat >"$tmp/clob.c" <<'EOF'
#include <stdio.h>
__int128 g = ((__int128)5 << 64) | 6;
__attribute__((noinline)) long f(long a, long b, long c) {
  long x = a * b + c, y = a ^ c, z = b - a;
  __int128 v = __atomic_load_n(&g, __ATOMIC_SEQ_CST);
  return x + y * 3 + z * 7 + (long)v + (long)(v >> 64);
}
__attribute__((noinline)) double fd(double a, double b, long n) {
  double x = a * b + 1.5, y = a - b;
  __atomic_store_n(&g, (__int128)n, __ATOMIC_SEQ_CST);
  return x * 2.0 + y;
}
__attribute__((noinline)) long fr(long a, long b) {
  long x = a * 7 + b, y = a ^ b;
  __int128 old = __atomic_fetch_add(&g, (__int128)1, __ATOMIC_SEQ_CST);
  return x + y * 5 + (long)old;
}
__attribute__((noinline)) long fc(long a, long b) {
  long x = a * 9 + b, y = b - a;
  __int128 e = (__int128)0, d = (__int128)a;
  int ok = __atomic_compare_exchange_n(&g, &e, d, 0, __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST);
  return x + y * 11 + ok + (long)e;
}
int main(void) {
  long r1 = f(3, 4, 5);
  double r2 = fd(2.0, 0.5, 0);
  long r3 = fr(6, 2);
  long r4 = fc(2, 9);
  printf("%ld %.2f %ld %ld\n", r1, r2, r3, r4);
  return 0;
}
EOF
# Two blocks of store + compare-exchange whose results feed inlined i128
# arithmetic (the reproducer that used to diverge at -O2 in both the
# libatomic and the inline cmpxchg16b configurations).
cat >"$tmp/mix.c" <<'EOF'
#include <stdio.h>
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
        MIX(e); MIX(__atomic_load_n(&v, __ATOMIC_SEQ_CST)); }
    {   __atomic_store_n(&v, k + 2, __ATOMIC_RELAXED);
        u128 e = k + 2;
        MIX((u128)__atomic_compare_exchange_n(&v, &e, k + 2 + (u128)1, 0,
                                              __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST));
        MIX(e); MIX(__atomic_load_n(&v, __ATOMIC_SEQ_CST)); }
    printf("%016llx\n", h);
    return 0;
}
EOF
for opt in -O1 -O2 -O3; do
    for flags in "" "-mcx16"; do
        lbl="$opt ${flags:-default}"
        "$ccc" $opt $flags "$tmp/clob.c" -o "$tmp/clob" -latomic || fail "lccc $lbl build of clob.c"
        out=$("$tmp/clob") || fail "lccc $lbl clob runtime"
        [ "$out" = "53 6.50 64 105" ] || fail "lccc $lbl: value clobbered across a 16-byte atomic: '$out'"
        "$ccc" $opt $flags "$tmp/mix.c" -o "$tmp/mix" -latomic || fail "lccc $lbl build of mix.c"
        out=$("$tmp/mix") || fail "lccc $lbl mix runtime"
        [ "$out" = "d674e05bde401cdc" ] || fail "lccc $lbl: i128 atomic results miscombined: '$out'"
    done
done
echo "PASS: no caller-saved home spans a 16-byte atomic (-O1/-O2/-O3, libatomic and cmpxchg16b)"

# ---------------------------------------------------------- oracle parity ----
if command -v "$gcc" >/dev/null 2>&1; then
    "$gcc" -O2 -mcx16 "$tmp/run.c" -o "$tmp/run_gcc" -latomic 2>/dev/null \
        || fail "$gcc cannot build the requested oracle"
    gout=$("$tmp/run_gcc") || fail "oracle run failed: $gout"
    "$ccc" -O2 -mcx16 "$tmp/run.c" -o "$tmp/run_l2" -latomic
    lout=$("$tmp/run_l2")
    # Compare the observable state, not the compiler's own prefix.
    [ "${gout#*: }" = "${lout#*: }" ] \
        || fail "oracle mismatch: $gcc='$gout' lccc='$lout'"
    echo "PASS: 16-byte atomic semantics match $gcc"
else
    echo "SKIP: $gcc not found; oracle parity not exercised" >&2
fi

echo "PASS: 16-byte atomic load/store/RMW (libatomic lowering, full 128-bit semantics)"
