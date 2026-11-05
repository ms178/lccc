/* `c ? 0 : 1` over a signed comparison is the inverted comparison -- of the
 * SAME operands, at the SAME operand type.
 *
 * RISC-V -O1 and up: predicate materialization replaced `x < 0 ? 0u : 1u`
 * by `x >= 0` but restated the comparison at the select's unsigned type.
 * The RISC-V backend extends a comparison's operands by that type, so it
 * zero-extended x and then compared signed: every negative x answered 1.
 * x86 and AArch64 compare the 32-bit registers directly and were right by
 * accident.  The cross harness runs this on AArch64 and RV64 at every
 * level; the native corpus on x86-64.
 */
typedef unsigned u32;
typedef unsigned long long u64;

__attribute__((noinline)) u32 nonneg(int x) { return x < 0 ? 0u : 1u; }
__attribute__((noinline)) u32 above_m3(int x) { return x > -3 ? 0u : 1u; }
__attribute__((noinline)) u32 le_m7(int x) { return x <= -7 ? 0u : 1u; }
__attribute__((noinline)) u64 nonneg64(long long x) { return x < 0 ? 0ull : 1ull; }
__attribute__((noinline)) u32 short_ge(short x) { return x >= -100 ? 0u : 1u; }
__attribute__((noinline)) int unsigned_le(u32 a, u32 b) { return a <= b ? 0 : 1; }
__attribute__((noinline)) u32 pair_lt(int a, int b) { return a < b ? 0u : 1u; }

int main(void)
{
    static volatile int xs[] = {-2147483647 - 1, -100, -7, -3, -1, 0, 1, 2, 2147483647};
    int bad = 0;
    for (unsigned i = 0; i < sizeof xs / sizeof *xs; i++) {
        int x = xs[i];
        if (nonneg(x) != (u32)!(x < 0))
            bad |= 1;
        if (above_m3(x) != (u32)!(x > -3))
            bad |= 2;
        if (le_m7(x) != (u32)!(x <= -7))
            bad |= 4;
        if (nonneg64((long long)x * 3) != (u64)!((long long)x * 3 < 0))
            bad |= 8;
        short s = (short)x;
        if (short_ge(s) != (u32)!(s >= -100))
            bad |= 16;
        if (unsigned_le((u32)x, 5u) != !((u32)x <= 5u))
            bad |= 32;
        if (pair_lt(x, -1) != (u32)!(x < -1))
            bad |= 64;
    }
    return bad;
}
