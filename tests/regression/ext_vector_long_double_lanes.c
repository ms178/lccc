/*
 * ABI-EXTVEC-LD-1: `long double` ext vectors by value on x86-64.
 *
 * Clang hands a vector return of class MEMORY to LLVM "direct" (a vector is
 * not an aggregate for its ABI code), so `<1 x x86_fp80>` returns in %st(0)
 * (exactly GCC's `struct { long double }`, X87 class) and `<2 x x86_fp80>`
 * returns in %st(0)/%st(1) (exactly GCC's `_Complex long double`); both
 * are MEMORY arguments at their full alignment (16 / 32).
 *
 * Before the fix the one-lane DEFINITION was lowered with an I64 return
 * (`get_function_return_type` collapsed every x86-64 vector return to I64
 * after the signature had already chosen F128), so lccc's own caller popped
 * %st(0) while the callee had converted the value to a double in %rax; the
 * two-lane return was a hard compile error. GCC ignores `ext_vector_type`
 * so the test is lccc-only and self-checking: every result is compared
 * against the same arithmetic on scalars (the vector is lane-wise), with
 * one exit bit per check. check_gnu_vector_align_abi.sh links the same
 * shapes against GCC's x87 struct / complex conventions.
 */
#include <stdio.h>

typedef long double T1 __attribute__((ext_vector_type(1)));
typedef long double T2 __attribute__((ext_vector_type(2)));
#define L0(v) ((v)[0])
#define L1(v) ((v)[1])
#define MK1(a) ((T1){a})
#define MK2(a, b) ((T2){a, b})

__attribute__((noinline)) T1 ident1(T1 a) { return a; }
__attribute__((noinline)) T1 kfour1(int k, T1 a, T1 b, T1 c, T1 d, int j) {
    T1 r = d;
    L0(r) = L0(a) + L0(b) * 2 + L0(c) * 4 + L0(d) * 8 + k + j;
    return r;
}
__attribute__((noinline)) long double sum1(int k, T1 a, long double x, T1 b) {
    return L0(a) * 3 + x + L0(b) * 7 + k;
}
__attribute__((noinline)) T2 ident2(T2 a) { return a; }
__attribute__((noinline)) T2 mk2(long double x, long double y) {
    T2 r;
    L0(r) = x * 2;
    L1(r) = y + 1;
    return r;
}
__attribute__((noinline)) T2 swap_add(int k, T2 a, T2 b) {
    T2 r;
    L0(r) = L1(b) + k;
    L1(r) = L0(a) - k;
    return r;
}
__attribute__((noinline)) long double hash2(int k, T2 a, T2 b, int j) {
    return ((k * 31 + L0(a)) * 31 + L1(a)) * 31 * 31 + L0(b) * 31 + L1(b) + j;
}
/* A deeper stack frame so the calls run on the other residue mod 32. */
__attribute__((noinline)) long double shifted(int depth) {
    volatile char pad[16];
    pad[0] = (char)depth;
    T2 a = MK2(1.25L, -3.5L), b = MK2(7.0L, 9.0L);
    return hash2(depth, a, b, 9) + pad[0] - depth;
}

int main(void) {
    int bad = 0;
    T1 a = MK1(1.25L), b = MK1(2.5L), c = MK1(4.0L), d = MK1(8.0L);
    if (L0(ident1(a)) != 1.25L) bad |= 1;
    if (L0(kfour1(3, a, b, c, d, 5)) != 1.25L + 2.5L * 2 + 4.0L * 4 + 8.0L * 8 + 3 + 5) bad |= 2;
    if (sum1(2, a, 0.5L, b) != 1.25L * 3 + 0.5L + 2.5L * 7 + 2) bad |= 4;
    T2 p = MK2(1.25L, -3.5L), q = MK2(7.0L, 9.0L);
    T2 r = ident2(p);
    if (L0(r) != 1.25L || L1(r) != -3.5L) bad |= 8;
    r = mk2(4.0L, 0.5L);
    if (L0(r) != 8.0L || L1(r) != 1.5L) bad |= 16;
    r = swap_add(3, p, q);
    if (L0(r) != 9.0L + 3 || L1(r) != 1.25L - 3) bad |= 32;
    long double h = ((5 * 31 + 1.25L) * 31 + -3.5L) * 31 * 31 + 7.0L * 31 + 9.0L + 9;
    if (hash2(5, p, q, 9) != h) bad |= 64;
    long double h1 = ((1 * 31 + 1.25L) * 31 + -3.5L) * 31 * 31 + 7.0L * 31 + 9.0L + 9;
    long double h2 = ((2 * 31 + 1.25L) * 31 + -3.5L) * 31 * 31 + 7.0L * 31 + 9.0L + 9;
    if (shifted(1) != h1 || shifted(2) != h2) bad |= 128;
    /* a chain: the pair return feeds the next call's memory argument */
    r = swap_add(1, mk2(2.0L, 3.0L), ident2(q));
    if (L0(r) != 9.0L + 1 || L1(r) != 4.0L - 1) bad |= 256;
    /* through a function pointer: the indirect-call return type is F128 */
    T2 (*volatile fp2)(long double, long double) = mk2;
    T1 (*volatile fp1)(T1) = ident1;
    r = fp2(4.0L, 0.5L);
    if (L0(r) != 8.0L || L1(r) != 1.5L) bad |= 512;
    if (L0(fp1(b)) != 2.5L) bad |= 1024;
    printf("ld lanes: %s (0x%x)\n", bad ? "FAIL" : "ok", bad);
    return bad;
}
