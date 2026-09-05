/*
 * The ABI static-chain register must not be an allocatable home in a function
 * that makes a direct nested-function call.
 *
 * `SetStaticChain` is lowered as a DIRECT write of the chain register (`%r10`
 * on x86-64, `%ecx` on i686) immediately before the call. It carries no IR
 * dest, so nothing told the linear scan that the register is redefined there,
 * and any value homed in it that was still live across the call was silently
 * destroyed.
 *
 * gcc.c-torture/execute/920501-7.c caught the x86-64 case at -O1..-Os: in the
 * recursive nested function the argument `a - 1` was homed in `%r10`, the
 * chain write clobbered it, and the relay peephole then legitimately rewrote
 * the argument copy to read the chain register, so the recursion was called
 * with a pointer instead of the decremented counter and never terminated.
 *
 *     movl 48(%rsp), %r10d
 *     subl $1, %r10d          # a-1 lives in %r10
 *     movq %r11, %r10         # SetStaticChain -- clobbers a-1
 *     movq %r11, %rdi         # arg0 reads the chain: y(chain), not y(a-1)
 *
 * The shapes below all put real register pressure across a nested call, so a
 * value WILL land in the chain register if it is left in the pool: several
 * live values crossing the call, a loop that keeps state across it, and a
 * non-local goto (which additionally needs the chain itself to survive).
 */
#include <stdio.h>

extern int printf(const char *, ...);

/* --- 1. the 920501-7 shape: recursion + non-local goto ---------------- */
__attribute__((noinline)) int nlg_depth(int a) {
    __label__ done;
    void rec(int n) {
        if (n == 0)
            goto done;
        rec(n - 1);
    }
    rec(a);
done:;
    return a;
}

/* --- 2. many live values across a direct nested call ------------------ */
__attribute__((noinline)) long pressure(long a, long b, long c, long d, long e) {
    long acc = 0;
    void bump(long k) { acc += k; }
    /* Every one of a..e must survive the chain-register write. */
    bump(a);
    bump(b);
    bump(c);
    bump(d);
    bump(e);
    return acc + a * 2 + b * 3 + c * 5 + d * 7 + e * 11;
}

/* --- 3. loop-carried state across a nested call ----------------------- */
__attribute__((noinline)) long loop_across_chain(const long *v, int n) {
    long sum = 0, prod = 1;
    void step(long x) {
        sum += x;
        prod = prod * 2 + (x & 1);
    }
    for (int i = 0; i < n; ++i) {
        long scaled = v[i] * 3 + i;
        step(scaled);
        sum += scaled ^ i; /* `scaled` and `i` must survive the call */
    }
    return sum * 1000 + prod;
}

/* --- 4. nested call in a conditional, chain reused afterwards --------- */
__attribute__((noinline)) int branchy(int a, int b) {
    int hits = 0;
    void note(int k) { hits += k; }
    if (a > b)
        note(a - b);
    else
        note(b - a);
    note(a & b);
    return hits * 10 + (a > b ? 1 : 2);
}

int main(void) {
    int fails = 0;
    long v[6] = {3, -4, 11, 0, 7, -2};

    for (int d = 0; d <= 6; ++d) {
        int got = nlg_depth(d);
        if (got != d) {
            printf("nlg_depth(%d) = %d\n", d, got);
            ++fails;
        }
    }

    {
        long got = pressure(1, 2, 3, 4, 5);
        long want = (1 + 2 + 3 + 4 + 5) + 1 * 2 + 2 * 3 + 3 * 5 + 4 * 7 + 5 * 11;
        if (got != want) {
            printf("pressure = %ld want %ld\n", got, want);
            ++fails;
        }
    }

    {
        long sum = 0, prod = 1;
        for (int i = 0; i < 6; ++i) {
            long scaled = v[i] * 3 + i;
            sum += scaled;
            prod = prod * 2 + (scaled & 1);
            sum += scaled ^ i;
        }
        long want = sum * 1000 + prod;
        long got = loop_across_chain(v, 6);
        if (got != want) {
            printf("loop_across_chain = %ld want %ld\n", got, want);
            ++fails;
        }
    }

    {
        struct { int a, b, want; } cases[] = {
            {9, 4, (9 - 4 + (9 & 4)) * 10 + 1},
            {4, 9, (9 - 4 + (4 & 9)) * 10 + 2},
            {7, 7, (0 + (7 & 7)) * 10 + 2},
        };
        for (unsigned k = 0; k < sizeof cases / sizeof cases[0]; ++k) {
            int got = branchy(cases[k].a, cases[k].b);
            if (got != cases[k].want) {
                printf("branchy(%d,%d) = %d want %d\n", cases[k].a, cases[k].b, got,
                       cases[k].want);
                ++fails;
            }
        }
    }

    printf("nested_fn_static_chain_regalloc: %s\n", fails ? "FAIL" : "OK");
    return fails != 0;
}
