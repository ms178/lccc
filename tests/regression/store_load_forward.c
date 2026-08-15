/* store_load_forward correctness battery: every shape that must NOT be
 * forwarded plus the profitable shapes that must stay correct.
 * Derived from the failure modes of the aggregate_copy_forward bug family
 * (8 unsoundness bugs) applied to the new pass. */
#include <stdio.h>
#include <string.h>

struct P { long a, b; };

/* 1. escape via pointer: callee writes through the escaped address */
static void __attribute__((noinline)) poke(long *p) { *p = 99; }
static long esc(void) {
    struct P s;
    s.a = 1;
    poke(&s.a);       /* address escapes, store must not forward across */
    return s.a;       /* must be 99 */
}

/* 2. join with disagreeing values */
static long __attribute__((noinline)) join(int c) {
    struct P s;
    if (c) s.a = 10; else s.a = 20;
    return s.a;
}

/* 3. overlapping narrow store kills wide entry */
static long overlap(void) {
    struct P s;
    s.a = 0x1111111111111111L;
    ((char *)&s.a)[3] = 0x22;
    return s.a;       /* byte 3 replaced */
}

/* 4. memcpy into the aggregate kills entries */
static long thrucpy(void) {
    struct P s, t;
    s.a = 7; s.b = 8;
    t.a = 100; t.b = 200;
    memcpy(&s, &t, sizeof s);
    return s.a + s.b; /* 300 */
}

/* 5. profitable case: build-then-read chain across blocks */
static long __attribute__((noinline)) build(int c) {
    struct P s;
    s.a = 40;
    s.b = 2;
    if (c) s.b = 3;
    return s.a + s.b; /* forwardable: s.a agreed on all paths */
}

/* 6. variable index store into the same aggregate */
static long varidx(int i) {
    long arr[4] = {1, 2, 3, 4};
    arr[0] = 5;
    arr[i] = 9;       /* variable GEP: kills tracking of arr */
    return arr[0];    /* 9 when i==0! */
}

int main(void) {
    if (esc() != 99) { printf("FAIL esc\n"); return 1; }
    if (join(1) != 10 || join(0) != 20) { printf("FAIL join\n"); return 2; }
    long ov = overlap();
    if (((ov >> 24) & 0xff) != 0x22) { printf("FAIL overlap %lx\n", ov); return 3; }
    if (thrucpy() != 300) { printf("FAIL thrucpy\n"); return 4; }
    if (build(0) != 42 || build(1) != 43) { printf("FAIL build\n"); return 5; }
    if (varidx(0) != 9 || varidx(1) != 5) { printf("FAIL varidx\n"); return 6; }
    printf("OK\n");
    return 0;
}
