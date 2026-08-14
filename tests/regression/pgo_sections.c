// PGO sections + alignment roundtrip:
//
// A skewed unit with three dominant handlers, ten lower-frequency handlers,
// and hot loops. The profile-use build must emit hot and unlikely function
// sections plus loop-header alignment while preserving the training output.
// This covers section ratios, single-block sectioning, block alignment, and
// alignment-aware jump relaxation.
#include <stdio.h>
#include <stdlib.h>

typedef unsigned long long u64;
static u64 state;

static u64 h_add(u64 a) { state += a; return state; }
static u64 h_mul(u64 a) { state *= (a | 1); return state; }
static u64 h_xor(u64 a) { state ^= a; return state; }
static u64 c_shl(u64 a) { return state << (a & 63); }
static u64 c_shr(u64 a) { return state >> (a & 63); }
static u64 c_rot(u64 a) { a &= 63; return (state << a) | (state >> (64 - a)); }
static u64 c_and(u64 a) { return state &= a; }
static u64 c_or(u64 a) { return state |= a; }
static u64 c_not(u64 a) { return state = ~state; }
static u64 c_neg(u64 a) { return state = 0 - state; }
static u64 c_bsf(u64 a) { return state = __builtin_ctzll(a); }
static u64 c_bsr(u64 a) { return state = 63 - __builtin_clzll(a); }
static u64 c_par(u64 a) { return state = __builtin_parityll(a); }

typedef u64 (*opfn)(u64);
static opfn ops[13] = { h_add, h_mul, h_xor, c_shl, c_shr, c_rot, c_and,
                        c_or, c_not, c_neg, c_bsf, c_bsr, c_par };

/* A hot loop that must be a loop header in the final codegen (align target). */
__attribute__((noinline)) static u64 spin(u64 a, int n) {
    for (int i = 0; i < n; i++) a = a * 2654435761u + i;
    return a;
}

int main(int argc, char **argv) {
    int n = argc > 1 ? atoi(argv[1]) : 3000000;
    u64 a = 0x9e3779b97f4a7c15ULL;
    state = a;
    for (int i = 0; i < n; i++) {
        int k = i % 100;
        int op;
        if (k < 30) op = 0; else if (k < 60) op = 1; else if (k < 90) op = 2;
        else op = 3 + (k - 90);
        a = ops[op](a + (u64)i);
    }
    printf("%llu\n", spin(a, 100000));
    return 0;
}
