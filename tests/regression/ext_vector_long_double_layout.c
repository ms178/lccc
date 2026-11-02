/*
 * In-memory layout of `long double` ext vectors (ABI-EXTVEC-LD-1, the lane
 * stride half of it). Clang lowers the vector to LLVM's `<N x x86_fp80>`,
 * whose elements are packed at their 80-bit SIZE: lane i starts at byte
 * 10*i. The C object is still Clang's AST size, N * sizeof(long double)
 * rounded up to a power of two (16/32/64), which is why a Clang `ld3`
 * global is a 32-byte ELF object inside a 64-byte C object. lccc used a
 * 16-byte stride (inferred from sizeof alone), so every multi-lane
 * `long double` vector disagreed with Clang in memory and across calls.
 *
 * An F128 store writes 16 bytes; a lane store must write only the 80-bit
 * image or it clobbers the next lane -- so lanes are written in every order
 * here. lccc-only (GCC ignores ext_vector_type); exit bit per check.
 */
#include <stdio.h>
#include <string.h>

typedef long double T1 __attribute__((ext_vector_type(1)));
typedef long double T2 __attribute__((ext_vector_type(2)));
typedef long double T3 __attribute__((ext_vector_type(3)));
typedef long double T4 __attribute__((ext_vector_type(4)));

_Static_assert(sizeof(T1) == 16 && sizeof(T2) == 32 && sizeof(T3) == 64 && sizeof(T4) == 64,
               "object sizes follow Clang's AST");

T3 g3 = {1.0L, 2.0L, 3.0L};
T4 g4 = {1.0L, 2.0L, 3.0L, 4.0L};

static long double lane_at(const void *obj, int i) {
    long double v;
    memcpy(&v, (const unsigned char *)obj + 10 * i, 10);
    return v;
}

__attribute__((noinline)) static T3 build(long double a, long double b, long double c, int order) {
    T3 r;
    switch (order) {
    case 0: r[0] = a; r[1] = b; r[2] = c; break;
    case 1: r[2] = c; r[1] = b; r[0] = a; break;
    default: r[1] = b; r[0] = a; r[2] = c; r[1] = b; break;
    }
    return r;
}

int main(void) {
    int bad = 0;
    if (lane_at(&g3, 1) != 2.0L || lane_at(&g3, 2) != 3.0L) bad |= 1;
    if (lane_at(&g4, 1) != 2.0L || lane_at(&g4, 3) != 4.0L) bad |= 2;
    T3 v = {1.0L, 2.0L, 3.0L};
    if (lane_at(&v, 0) != 1.0L || lane_at(&v, 1) != 2.0L || lane_at(&v, 2) != 3.0L) bad |= 4;
    for (int order = 0; order < 3; order++) {
        T3 w = build(10.0L, 20.0L, 30.0L, order);
        if (w[0] != 10.0L || w[1] != 20.0L || w[2] != 30.0L) bad |= 8 << order;
        if (lane_at(&w, 1) != 20.0L || lane_at(&w, 2) != 30.0L) bad |= 64 << order;
    }
    /* a lane written from another lane's 16-byte load must not leak padding */
    T3 c = v;
    c[1] = c[0];
    c[0] = c[2];
    if (c[0] != 3.0L || c[1] != 1.0L || c[2] != 3.0L) bad |= 1 << 9;
    T3 s = v + c, m = v * c;
    if (s[0] != 4.0L || s[1] != 3.0L || s[2] != 6.0L) bad |= 1 << 10;
    if (m[0] != 3.0L || m[1] != 2.0L || m[2] != 9.0L) bad |= 1 << 11;
    T2 p = {5.0L, 6.0L}, q = p;
    q[1] = p[0];
    q[0] = p[1];
    if (q[0] != 6.0L || q[1] != 5.0L || lane_at(&q, 1) != 5.0L) bad |= 1 << 12;
    volatile T2 vv = p;
    vv[1] = 9.0L;
    if (vv[0] != 5.0L || vv[1] != 9.0L) bad |= 1 << 13;
    T1 one = {7.0L};
    one[0] += 1;
    if (one[0] != 8.0L || lane_at(&one, 0) != 8.0L) bad |= 1 << 14;
    /* the precision must be the full 64-bit mantissa, lane by lane */
    T2 fine = {1.0L + 0x1p-60L, 0x1p-62L};
    if (fine[0] + fine[1] * 3 != (1.0L + 0x1p-60L) + 0x1p-62L * 3) bad |= 1 << 15;
    printf("ld layout: %s (0x%x)\n", bad ? "FAIL" : "ok", bad);
    return bad != 0; /* an exit status keeps 8 bits: `return bad` hid bits 8+ */
}
