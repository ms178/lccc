/* Highest-set-bit index with -1 for zero (`x ? (W-1) - clz(x) : -1`),
 * consumed at a wider type. x86-64 folds the idiom to
 * `mov $-1, %D; bsr %S, %D` (BSR leaves the destination unchanged for a
 * zero source), so for x == 0 the register holds the -1 preload rather than
 * a bit count. The re-extension that follows must therefore survive the
 * redundant-extension pass: `(long)hb(0)` is -1, never 4294967295.
 * The in-range passthrough idioms (`x ? clz(x) : W`, `x ? W - clz(x) : 0`)
 * are mixed in so both preload classes run at zero and nonzero inputs.
 * Differential (lccc vs gcc); every expression is defined for every input. */
#include <stdio.h>
#include <stdint.h>

#define NI __attribute__((noinline))

NI int hb32(unsigned x) { return x ? 31 - __builtin_clz(x) : -1; }
NI int hb64(uint64_t x) { return x ? 63 - __builtin_clzll(x) : -1; }
NI long hb32_l(unsigned x) { int r = x ? 31 - __builtin_clz(x) : -1; return r; }
NI long hb64_l(uint64_t x) { int r = x ? 63 - __builtin_clzll(x) : -1; return r; }
NI long long hb64_ll(uint64_t x) { return x ? 63 - __builtin_clzll(x) : -1LL; }
NI unsigned long hb32_ul(unsigned x) { int r = x ? 31 - __builtin_clz(x) : -1; return (unsigned long)(long)r; }
NI long hb32_sc(unsigned x) { signed char r = x ? 31 - __builtin_clz(x) : -1; return r; }
NI long lowbit_l(unsigned x) { int r = x ? __builtin_ctz(x) : -1; return r; }
NI long fls32_l(unsigned x) { int r = x ? 32 - __builtin_clz(x) : 0; return r; }
NI long lz32_l(unsigned x) { int r = x ? __builtin_clz(x) : 32; return r; }
NI long tz64_l(uint64_t x) { int r = x ? __builtin_ctzll(x) : 64; return r; }

static const uint64_t inputs[] = {
    0, 1, 2, 3, 0x80, 0x7fffffff, 0x80000000u, 0xffffffffu,
    0x100000000ull, 0x8000000000000000ull, 0xffffffffffffffffull, 0x123456789abcdefull,
};

int main(void) {
    long acc = 0;
    for (unsigned i = 0; i < sizeof inputs / sizeof inputs[0]; i++) {
        uint64_t x = inputs[i];
        unsigned x32 = (unsigned)x;
        long v[] = {
            (long)hb32(x32), (long)hb64(x), hb32_l(x32), hb64_l(x), (long)hb64_ll(x),
            (long)hb32_ul(x32), hb32_sc(x32), lowbit_l(x32), fls32_l(x32), lz32_l(x32),
            tz64_l(x),
        };
        printf("%#llx:", (unsigned long long)x);
        for (unsigned k = 0; k < sizeof v / sizeof v[0]; k++) {
            printf(" %ld", v[k]);
            acc = acc * 31 + v[k];
        }
        printf("\n");
    }
    /* A widened -1 in address arithmetic: a wrong zero-extension would
     * index 4 GiB past the array instead of the element before `base`. */
    static int table[4] = {10, 20, 30, 40};
    int *base = &table[1];
    acc += base[hb32_l(0)] + base[hb64_l(0)] + base[hb32_l(1)];
    printf("acc=%ld\n", acc);
    return 0;
}
