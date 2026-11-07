/* Sparse switches through the balanced case-cluster tree on AArch64 and
 * RISC-V (check_switch_cross_execute.py): every selector type the tree
 * takes (int, unsigned, long, unsigned long), negative and sign-boundary
 * case values, runs of consecutive values with one target (range tests,
 * including spans across zero and the 32-bit sign boundary), split values
 * that are cases of their own (the pivot compare answers both), pivots of
 * 0, and targets reached by several clusters.
 *
 * Each selector runs over every case value, its neighbours and the type's
 * extremes; the results fold into one value per function, compared with
 * what GCC 16.2 computes for the same C on x86-64 (`-DGEN` prints them).
 * main returns the 1-based index of the first mismatch.
 */
#ifdef GEN
#include <stdio.h>
#endif

#define NI __attribute__((noinline))

NI int sparse_int(int x)
{
    switch (x) {
    case -2147483647 - 1: return 1;
    case -100000: return 2;
    case -7: return 3;
    case -1: return 4;
    case 0: return 5;
    case 10: return 6;
    case 100: return 7;
    case 1000: return 8;
    case 10000: return 9;
    case 2147483647: return 10;
    default: return 0;
    }
}

NI int runs_int(int x)
{
    switch (x) {
    case -3: case -2: case -1: case 0: case 1: case 2: return 1;
    case 9: case 10: case 13: case 32: return 2;
    case 35: return 3;
    case 59: case 60: case 61: return 4;
    case 200: case 201: return 1;
    default: return 0;
    }
}

NI unsigned sparse_u32(unsigned x)
{
    switch (x) {
    case 0u: return 1;
    case 3u: return 2;
    case 1000u: return 3;
    case 0x7fffffffu: return 4;
    case 0x80000000u: return 5;
    case 0x80000001u: return 6;
    case 0xfffffff0u: case 0xfffffff1u: case 0xfffffff2u: return 7;
    case 0xffffffffu: return 8;
    default: return 9;
    }
}

NI int runs_u32_boundary(unsigned x)
{
    switch (x) {
    case 0x7ffffffeu: case 0x7fffffffu: case 0x80000000u: case 0x80000001u: return 1;
    case 5u: case 6u: return 2;
    case 70000u: return 3;
    case 0xfffffffeu: case 0xffffffffu: case 0u: return 4;
    default: return 0;
    }
}

NI long sparse_long(long x)
{
    switch (x) {
    case -0x7fffffffffffffffL - 1: return 1;
    case -0x100000000L: return 2;
    case -1: return 3;
    case 0: return 4;
    case 1: case 2: case 3: return 5;
    case 0x7fffffffL: return 6;
    case 0x80000000L: return 7;
    case 0x123456789abL: return 8;
    case 0x7fffffffffffffffL: return 9;
    default: return 0;
    }
}

NI int sparse_ulong(unsigned long x)
{
    switch (x) {
    case 0ul: return 1;
    case 1ul: case 2ul: return 2;
    case 0xfffffffful: return 3;
    case 0x100000000ul: return 4;
    case 0x7ffffffffffffffful: return 5;
    case 0x8000000000000000ul: case 0x8000000000000001ul: return 6;
    case 0xfffffffffffffffeul: return 7;
    case 0xfffffffffffffffful: return 8;
    default: return 0;
    }
}

/* Distinct bodies per case (no lookup table can serve it). */
NI long dispatch(int op, long a, long b)
{
    switch (op) {
    case 1: return a + b;
    case 2: return a - b;
    case 4: return a * 3;
    case 8: return b ^ 0x55;
    case 16: return a << 2;
    case 32: return b >> 1;
    case 64: return a | b;
    case 128: return a & b;
    default: return -1;
    }
}

static unsigned long mix(unsigned long h, long v)
{
    return (h ^ (unsigned long)v) * 0x100000001b3ul;
}

static const int int_probes[] = {
    -2147483647 - 1, -2147483647, -100001, -100000, -99999, -8, -7, -6, -4, -3, -2, -1, 0, 1,
    2, 3, 8, 9, 10, 11, 12, 13, 14, 31, 32, 33, 34, 35, 36, 58, 59, 60, 61, 62, 99, 100, 101,
    199, 200, 201, 202, 999, 1000, 1001, 9999, 10000, 10001, 2147483646, 2147483647,
};
static const unsigned u32_probes[] = {
    0u, 1u, 2u, 3u, 4u, 5u, 6u, 7u, 999u, 1000u, 1001u, 69999u, 70000u, 70001u,
    0x7ffffffdu, 0x7ffffffeu, 0x7fffffffu, 0x80000000u, 0x80000001u, 0x80000002u,
    0xffffffefu, 0xfffffff0u, 0xfffffff1u, 0xfffffff2u, 0xfffffff3u, 0xfffffffdu,
    0xfffffffeu, 0xffffffffu,
};
static const long long_probes[] = {
    -0x7fffffffffffffffL - 1, -0x7fffffffffffffffL, -0x100000001L, -0x100000000L,
    -0xffffffffL, -2, -1, 0, 1, 2, 3, 4, 0x7ffffffeL, 0x7fffffffL, 0x80000000L,
    0x80000001L, 0x123456789aaL, 0x123456789abL, 0x123456789acL, 0x7ffffffffffffffeL,
    0x7fffffffffffffffL,
};

#ifdef GEN
#define CHECK(e) printf("    0x%lxul,\n", (unsigned long)(e))
#else
/* Generated: gcc-16 -O2 -DGEN switch_tree_cross.c && ./a.out (identical
 * at -O0 and with Clang 23.1.5). */
static const unsigned long expected[] = {
    0xce74f4361b458491ul, 0x8e54363530d08053ul, 0x5b386b5542323668ul, 0x1d6fac72334f6c78ul,
    0xc6fb27439842451ful, 0x49fb6c69e5a393cdul, 0x968724a3cfd912e8ul, 0xfffffffffffffffful,
    0xcul, 0x0ul, 0xfffffffffffffffful, 0x4bul, 0xfffffffffffffffful, 0xfffffffffffffffful,
    0xfffffffffffffffful, 0x54ul, 0xfffffffffffffffful, 0xfffffffffffffffful,
    0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful,
    0xfffffffffffffffful, 0x1b4ul, 0xfffffffffffffffful, 0xfffffffffffffffful,
    0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful,
    0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful,
    0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful,
    0xfffffffffffffffful, 0x14ul, 0xfffffffffffffffful, 0xfffffffffffffffful,
    0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful,
    0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful,
    0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful,
    0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful,
    0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful,
    0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful,
    0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful,
    0xfffffffffffffffful, 0x1fdul, 0xfffffffffffffffful, 0xfffffffffffffffful,
    0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful,
    0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful,
    0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful,
    0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful,
    0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful,
    0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful,
    0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful,
    0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful,
    0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful,
    0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful,
    0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful,
    0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful,
    0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful,
    0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful,
    0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful,
    0xfffffffffffffffful, 0x9ul, 0xfffffffffffffffful, 0xfffffffffffffffful,
    0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful,
    0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful, 0xfffffffffffffffful,
    0xfffffffffffffffful,
};
/* The index of the next check; a debugger reads the failing one here. */
volatile int n__;
#define CHECK(e)                                     \
    do {                                             \
        if ((unsigned long)(e) != expected[n__])     \
            return n__ + 1;                          \
        n__ = n__ + 1;                               \
    } while (0)
#endif

int main(void)
{
    unsigned long h;
    int count = sizeof int_probes / sizeof int_probes[0];
    h = 0;
    for (int i = 0; i < count; i++)
        h = mix(h, sparse_int(int_probes[i]));
    CHECK(h);
    h = 0;
    for (int i = 0; i < count; i++)
        h = mix(h, runs_int(int_probes[i]));
    CHECK(h);
    h = 0;
    for (int i = 0; i < count; i++)
        h = mix(h, dispatch(int_probes[i] & 255, int_probes[i], i));
    CHECK(h);
    count = sizeof u32_probes / sizeof u32_probes[0];
    h = 0;
    for (int i = 0; i < count; i++)
        h = mix(h, sparse_u32(u32_probes[i]));
    CHECK(h);
    h = 0;
    for (int i = 0; i < count; i++)
        h = mix(h, runs_u32_boundary(u32_probes[i]));
    CHECK(h);
    count = sizeof long_probes / sizeof long_probes[0];
    h = 0;
    for (int i = 0; i < count; i++)
        h = mix(h, sparse_long(long_probes[i]));
    CHECK(h);
    h = 0;
    for (int i = 0; i < count; i++) {
        h = mix(h, sparse_ulong((unsigned long)long_probes[i]));
        h = mix(h, sparse_ulong((unsigned long)long_probes[i] + 0x8000000000000000ul));
    }
    CHECK(h);
    for (int op = 0; op < 140; op++)
        CHECK(dispatch(op, op * 7 - 3, op ^ 9));
    return 0;
}
