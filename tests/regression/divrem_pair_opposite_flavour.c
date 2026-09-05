/*
 * A fused div/rem pair must have OPPOSITE flavours.
 *
 * One `idivl`/`divl` yields the quotient in %eax and the remainder in %edx, so
 * fusing two div-like operations only makes sense when the two consumers want
 * DIFFERENT halves. Every backend emitter derives the register split from the
 * HEAD's flavour alone (i686 `emit_divrem_pair_head`:
 * `div_dest = if self_is_div { dest } else { partner }`; the AArch64 partner
 * map does not even carry the tail's flavour) — i.e. they all *assume* the
 * flavours differ. `compute_i686_divrem_pairs` never enforced it: it paired any
 * two div-likes in a block with identical operands and matching signedness.
 *
 * Two same-flavour operations with identical operands compute the SAME value
 * (a CSE miss, not a fusion opportunity), and the tail was then stored from the
 * wrong register:
 *
 *     x[n % 1000] = 2;      ->   movl %edi, %eax    # %edi held n / 1000
 *                                shll $2, %eax
 *                                addl -24(%ebp), %eax
 *                                movl $2, (%eax)    # index up to 999 into a
 *                                                   # (n % 1000 + 1)-element VLA
 *
 * On i686 this took out eight gcc.c-torture tests at -O0/-O1:
 * `20040811-1.c`, `vla-dealloc-1.c`, `pr43220.c` (repeated `n % 1000` against a
 * VLA -> stack scribble -> SIGSEGV), `981001-1.c` (repeated `n / 2`),
 * `20000511-1.c` (repeated `b % c`), and `ssad-run.c` / `usad-run.c`.
 *
 * Every shape below keeps two div-likes of the SAME operands in one basic block
 * so the pairing scan sees them, and checks the value against a `volatile`
 * reference the compiler cannot fold.
 */
#include <stdio.h>

extern int printf(const char *, ...);

volatile int v1000 = 1000, v2 = 2, v7 = 7;
volatile unsigned vu = 1000u;

/* --- 1. two SRems of the same operands (the VLA shape) ---------------- */
__attribute__((noinline)) int two_srem(int n, int d) {
    int a = n % d;
    int b = n % d;
    return a * 31 + b;
}

/* --- 2. two SDivs of the same operands (981001-1) --------------------- */
__attribute__((noinline)) int two_sdiv(int n, int d) {
    int a = n / d;
    int b = n / d;
    return a * 31 + b;
}

/* --- 3. two URems / two UDivs ----------------------------------------- */
__attribute__((noinline)) unsigned two_urem(unsigned n, unsigned d) {
    unsigned a = n % d;
    unsigned b = n % d;
    return a * 31u + b;
}
__attribute__((noinline)) unsigned two_udiv(unsigned n, unsigned d) {
    unsigned a = n / d;
    unsigned b = n / d;
    return a * 31u + b;
}

/* --- 4. three of a kind, then the genuine opposite-flavour pair ------- */
__attribute__((noinline)) int three_then_pair(int n, int d) {
    int a = n % d;
    int b = n % d;
    int c = n % d;
    int q = n / d; /* this one legitimately fuses with a remainder */
    return ((a + b + c) * 31 + q);
}

/* --- 5. the pair that SHOULD fuse still has to be correct ------------- */
__attribute__((noinline)) int div_then_rem(int n, int d) {
    int q = n / d;
    int r = n % d;
    return q * 1000 + r;
}
__attribute__((noinline)) int rem_then_div(int n, int d) {
    int r = n % d;
    int q = n / d;
    return q * 1000 + r;
}

/* --- 6. constant divisor (the magic-number head path) ----------------- */
__attribute__((noinline)) int two_srem_const(int n) {
    int a = n % 1000;
    int b = n % 1000;
    return a * 31 + b;
}
__attribute__((noinline)) int const_div_then_rem(int n) {
    int q = n / 1000;
    int r = n % 1000;
    return q * 100000 + r;
}

/* --- 7. the exact torture shape: repeated `n % 1000` indexing a VLA --- */
void *volatile keep;
__attribute__((noinline)) int vla_repeat_mod(int n) {
    int x[n % 1000 + 1];
    x[0] = 1;
    x[n % 1000] = 2; /* must be the REMAINDER, never the quotient */
    keep = x;
    return x[n % 1000] + x[0];
}

int main(void) {
    int fails = 0;
    const int ns[] = {0, 1, 7, 999, 1000, 1001, 123456, -1, -7, -1000, -123456};
    const int ds[] = {2, 3, 7, 1000};

    for (unsigned i = 0; i < sizeof ns / sizeof ns[0]; ++i) {
        for (unsigned k = 0; k < sizeof ds / sizeof ds[0]; ++k) {
            int n = ns[i], d = ds[k];
            volatile int vn = n, vd = d;
            int wr = vn % vd, wq = vn / vd;

            if (two_srem(n, d) != wr * 31 + wr) {
                printf("two_srem(%d,%d) = %d want %d\n", n, d, two_srem(n, d), wr * 31 + wr);
                ++fails;
            }
            if (two_sdiv(n, d) != wq * 31 + wq) {
                printf("two_sdiv(%d,%d) = %d want %d\n", n, d, two_sdiv(n, d), wq * 31 + wq);
                ++fails;
            }
            if (three_then_pair(n, d) != (wr + wr + wr) * 31 + wq) {
                printf("three_then_pair(%d,%d) = %d want %d\n", n, d, three_then_pair(n, d),
                       (wr + wr + wr) * 31 + wq);
                ++fails;
            }
            if (div_then_rem(n, d) != wq * 1000 + wr) {
                printf("div_then_rem(%d,%d) = %d want %d\n", n, d, div_then_rem(n, d),
                       wq * 1000 + wr);
                ++fails;
            }
            if (rem_then_div(n, d) != wq * 1000 + wr) {
                printf("rem_then_div(%d,%d) = %d want %d\n", n, d, rem_then_div(n, d),
                       wq * 1000 + wr);
                ++fails;
            }
        }
    }

    {
        const unsigned uns[] = {0u, 1u, 999u, 1000u, 4000000000u};
        const unsigned uds[] = {3u, 1000u, 65537u};
        for (unsigned i = 0; i < sizeof uns / sizeof uns[0]; ++i)
            for (unsigned k = 0; k < sizeof uds / sizeof uds[0]; ++k) {
                volatile unsigned un = uns[i], ud = uds[k];
                unsigned wr = un % ud, wq = un / ud;
                if (two_urem(uns[i], uds[k]) != wr * 31u + wr) {
                    printf("two_urem(%u,%u) wrong\n", uns[i], uds[k]);
                    ++fails;
                }
                if (two_udiv(uns[i], uds[k]) != wq * 31u + wq) {
                    printf("two_udiv(%u,%u) wrong\n", uns[i], uds[k]);
                    ++fails;
                }
            }
    }

    for (unsigned i = 0; i < sizeof ns / sizeof ns[0]; ++i) {
        int n = ns[i];
        volatile int vn = n;
        int wr = vn % 1000, wq = vn / 1000;
        if (two_srem_const(n) != wr * 31 + wr) {
            printf("two_srem_const(%d) = %d want %d\n", n, two_srem_const(n), wr * 31 + wr);
            ++fails;
        }
        if (const_div_then_rem(n) != wq * 100000 + wr) {
            printf("const_div_then_rem(%d) = %d want %d\n", n, const_div_then_rem(n),
                   wq * 100000 + wr);
            ++fails;
        }
    }

    /* The VLA shape, over a range that makes a quotient index fatal.
     * When n % 1000 == 0 both stores hit x[0], so the expected sum is 4. */
    for (int n = 0; n < 5000; n += 137) {
        int want = (n % 1000 == 0) ? 4 : 3;
        int got = vla_repeat_mod(n);
        if (got != want) {
            printf("vla_repeat_mod(%d) = %d want %d\n", n, got, want);
            ++fails;
            break;
        }
    }

    printf("divrem_pair_opposite_flavour: %s\n", fails ? "FAIL" : "OK");
    return fails != 0;
}
