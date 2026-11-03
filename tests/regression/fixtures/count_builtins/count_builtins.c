/*
 * Freestanding four-target differential for the integer count builtins:
 * __builtin_clz/ctz and their `l`/`ll` variants.
 *
 * The references are plain shift loops compiled into the same image, so a
 * wrong count lowering cannot cancel itself out; the operands arrive through
 * `volatile` so the counts must come from the backend's real lowering (a
 * constant folder that answers the builtin itself would hide the defect).
 *
 * Every count is checked for the defined-at-zero contract the IR guarantees --
 * Clz(0)/Ctz(0) == the operand's bit width -- which is the case the per-target
 * fallbacks get wrong: `bsr`/`bsf` leave their destination undefined there, and
 * a RISC-V software scan has to reach that answer without a special case.
 *
 * `test()` follows the freestanding entry stubs the runtime gates share
 * (start-<target>.s): it returns 0 on success and a non-zero class mask
 * otherwise, so a failure is visible as a non-zero exit status under QEMU.
 */

typedef unsigned int u32;
typedef unsigned long long u64;

static unsigned long long fails;
/* Bit per failing kernel, so the exit status names what broke. */
#define K_CLZ32 1u
#define K_CTZ32 2u
#define K_CLZ64 4u
#define K_CTZ64 8u
#define K_CLZL 16u
#define K_CTZL 32u

static void note(unsigned class_bit)
{
        fails |= class_bit;
}

static unsigned ref_clz32(u32 x)
{
        unsigned n = 0;
        if (!x)
                return 32;
        while (!(x & 0x80000000u)) {
                x <<= 1;
                n++;
        }
        return n;
}

static unsigned ref_ctz32(u32 x)
{
        unsigned n = 0;
        if (!x)
                return 32;
        while (!(x & 1u)) {
                x >>= 1;
                n++;
        }
        return n;
}

static unsigned ref_clz64(u64 x)
{
        unsigned n = 0;
        if (!x)
                return 64;
        while (!(x & 0x8000000000000000ull)) {
                x <<= 1;
                n++;
        }
        return n;
}

static unsigned ref_ctz64(u64 x)
{
        unsigned n = 0;
        if (!x)
                return 64;
        while (!(x & 1ull)) {
                x >>= 1;
                n++;
        }
        return n;
}

/* `long` is 32-bit on i686 and 64-bit elsewhere, so this reference pair is
 * written against its own width instead of reusing the u32/u64 forms. */
static unsigned ref_clzl(unsigned long x)
{
        unsigned n = 0;
        unsigned long top = 1ul << (sizeof(unsigned long) * 8 - 1);
        if (!x)
                return sizeof(unsigned long) * 8;
        while (!(x & top)) {
                x <<= 1;
                n++;
        }
        return n;
}

static unsigned ref_ctzl(unsigned long x)
{
        unsigned n = 0;
        if (!x)
                return sizeof(unsigned long) * 8;
        while (!(x & 1ul)) {
                x >>= 1;
                n++;
        }
        return n;
}

/* One out-of-line kernel per builtin: the builtin is lowered where it is
 * written, never re-optimized at a call site. */
static unsigned k_clz32(u32 x) { volatile u32 v = x; return __builtin_clz(v); }
static unsigned k_ctz32(u32 x) { volatile u32 v = x; return __builtin_ctz(v); }
static unsigned k_clz64(u64 x) { volatile u64 v = x; return __builtin_clzll(v); }
static unsigned k_ctz64(u64 x) { volatile u64 v = x; return __builtin_ctzll(v); }
static unsigned k_clzl(unsigned long x) { volatile unsigned long v = x; return __builtin_clzl(v); }
static unsigned k_ctzl(unsigned long x) { volatile unsigned long v = x; return __builtin_ctzl(v); }

static void check32(u32 x)
{
        if (k_clz32(x) != ref_clz32(x))
                note(K_CLZ32);
        if (k_ctz32(x) != ref_ctz32(x))
                note(K_CTZ32);
        if (k_clzl((unsigned long)x) != ref_clzl((unsigned long)x))
                note(K_CLZL);
        if (k_ctzl((unsigned long)x) != ref_ctzl((unsigned long)x))
                note(K_CTZL);
}

static void check64(u64 x)
{
        if (k_clz64(x) != ref_clz64(x))
                note(K_CLZ64);
        if (k_ctz64(x) != ref_ctz64(x))
                note(K_CTZ64);
}

static unsigned long long seed = 0x243f6a8885a308d3ull;

static u64 next(void)
{
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        return seed;
}

int test(void)
{
        u32 x;
        u64 y;
        unsigned k;

        /* Exhaustive over the low half-word: the whole zero/short-value range
         * of every 32-bit count, on every kernel. */
        for (x = 0; x <= 0xffffu; x++)
                check32(x);

        /* Every single-bit operand with both neighbours plus the complements,
         * which is where a mask or a shift count goes wrong. */
        for (k = 0; k < 32; k++) {
                u32 b = 1u << k;
                check32(b);
                check32(b - 1);
                check32(b + 1);
                check32(~b);
                check32(~(b - 1));
        }
        check32(0x80000000u);
        check32(0x7fffffffu);
        check32(0xffffffffu);
        check32(0xaaaaaaaau);
        check32(0x55555555u);

        for (k = 0; k < 64; k++) {
                u64 b = 1ull << k;
                check64(b);
                check64(b - 1);
                check64(b + 1);
                check64(~b);
                check64(~(b - 1));
        }
        check64(0);
        check64(0xffffffffffffffffull);
        check64(0x8000000000000000ull);
        check64(0x7fffffffffffffffull);
        check64(0xaaaaaaaaaaaaaaaaull);
        check64(0x5555555555555555ull);

        /* Random sweeps: unconstrained, narrow, and top-half-heavy operands. */
        for (k = 0; k < 100000; k++) {
                check32((u32)next());
                y = next();
                check64(y);
                check64(y & 0xff);
                check64(y >> 32);
                check64(0x8000000000000000ull | (y >> 8));
                check32((u32)(y & 0xffffffffu) & 0xffff0000u);
        }

        return (int)(fails & 0xffffffffull);
}
