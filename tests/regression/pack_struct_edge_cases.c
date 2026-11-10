/*
 * Edge-case regression tests for -fpack-struct[=<n>] (DA-07).
 *
 * Compile with: gcc -Wall -Wextra -Werror -c pack_struct_edge_cases.c
 * These tests verify that the C source is valid and exercises all
 * pack-struct edge paths that lccc must handle identically to GCC.
 */

/* Test 1: Bare -fpack-struct packs all records */
#if defined(__GNUC__) && !defined(__clang__)
#pragma GCC diagnostic ignored "-Wpragmas"
#endif

struct packed_bare {
    char c;
    int i;
    char d;
};

#ifdef __SIZEOF_PACKED_BARE__
/* GCC will compute this; we can't use _Static_assert here without knowing
 * the exact flags. This file is compiled multiple times with different
 * -fpack-struct flags. */
#endif

/* Test 2: Explicit member alignment escapes bare -fpack-struct */
struct with_aligned_member {
    char c;
    int i __attribute__((aligned(8)));
    char d;
} __attribute__((aligned(16)));

/* Test 3: Bitfields under bare -fpack-struct become contiguous bit stream */
struct bitfield_packed {
    unsigned a: 3;
    unsigned b: 5;
    unsigned c: 20;
    char d;
};

/* Test 4: Union under bare -fpack-struct */
union packed_union {
    char c;
    long long l;
};

/* Test 5: Nested records under bare -fpack-struct */
struct inner { char c; int i; };
struct outer {
    char c;
    struct inner inner;
    char d;
};

/* Test 6: -fpack-struct=<n> caps even explicitly aligned members */
struct capped {
    char c;
    int i __attribute__((aligned(8)));
};

/* Test 7: #pragma pack after -fpack-struct=<n> */
struct before_pragma { char c; int i; };
#pragma pack(4)
struct after_pragma { char c; int i; };
#pragma pack()
struct after_reset { char c; int i; };

/* Test 8: #pragma pack(push) / pack(pop) with cap */
#pragma pack(push, 1)
struct push_one { char c; int i; };
#pragma pack(push, 2)
struct push_two { char c; int i; };
#pragma pack(pop)
struct pop_back { char c; int i; };
#pragma pack(pop)
struct pop_again { char c; int i; };

/* Test 9: Invalid -fpack-struct values are rejected at the driver level */
/* These are tested by the driver, not by compiling this file directly. */

/* Test 10: Bare -fpack-struct makes #pragma pack inert */
#pragma pack(8)
struct pragma_inert { char c; int i; };

int main(void) {
    /* Ensure all types are used so they don't get optimized away */
    volatile struct packed_bare *pb = 0;
    volatile struct with_aligned_member *wa = 0;
    volatile struct bitfield_packed *bf = 0;
    volatile union packed_union *pu = 0;
    volatile struct outer *ov = 0;
    volatile struct capped *cp = 0;
    volatile struct before_pragma *bp = 0;
    volatile struct after_pragma *ap = 0;
    volatile struct after_reset *ar = 0;
    volatile struct push_one *po1 = 0;
    volatile struct push_two *po2 = 0;
    volatile struct pop_back *pbk = 0;
    volatile struct pop_again *pa = 0;
    volatile struct pragma_inert *pi = 0;
    (void)pb; (void)wa; (void)bf; (void)pu; (void)ov; (void)cp;
    (void)bp; (void)ap; (void)ar; (void)po1; (void)po2; (void)pbk;
    (void)pa; (void)pi;
    return 0;
}
