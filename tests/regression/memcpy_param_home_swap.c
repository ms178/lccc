/*
 * Regression: x86 memcpy/memmove address staging must be a PARALLEL copy.
 *
 * Root cause (mc2, 2026-09): the generic `emit_memcpy` trait default staged
 * `dest` and `src` into %rdi/%rsi *sequentially* through %rcx.  With
 * parameters kept in their ABI registers (`x86_param_caller_homes_safe`) a
 * function `f(struct S *src, struct S *dst)` has `src` homed in %rdi and
 * `dst` in %rsi.  Staging `dst → %rdi` first destroyed `src`; the copy then
 * ran with source == destination and became a no-op (the peephole even
 * collapsed the movq pair into `(%rsi) -> (%rsi)`).
 *
 * The x86 backend now overrides `emit_memcpy` with a two-node parallel-move
 * schedule (`stage_copy_operands`): cross-homes → `xchgq`, one-sided alias →
 * stage the aliased operand first, otherwise any order.  The same staging
 * is shared by `__builtin_memcpy` / `__builtin_memmove` inline expansions.
 *
 * Every function below is `noinline` so the parameter homes are decided by
 * the ABI, and each one exercises a distinct home permutation:
 *   f   src∈rdi, dst∈rsi           -> full swap (xchgq)
 *   g   src∈rdi, dst∈rdx           -> src must move before dst is staged
 *   g2  dst∈rdi, src∈rsi           -> identity (no moves)
 *   g3  dst∈rsi, src∈rdx           -> dst staged first would clobber nothing;
 *                                     src must not be read from rsi after
 *   h   long double (F128 fusion path in generation.rs) src∈rdi, dst∈rsi
 *   k   __int128 slot → pointer param (Direct slot address source)
 *   m   over-aligned alloca source, pointer dest in rdi
 *   mm  memmove overlap, both directions, src∈rdi
 *   many  sizes 1..64 through a swapped-home copy, so every ladder shape
 *         (movdqu pairs, movq, movl, movw, movb, rep movsb under -mno-sse)
 *         runs with the parallel staging.
 */
#include <stdio.h>
#include <string.h>

struct S { long a[6]; };
struct T { char c[13]; };

__attribute__((noinline)) void f(struct S *src, struct S *dst) { *dst = *src; }
__attribute__((noinline)) void g(struct S *src, long x, struct S *dst) { (void)x; *dst = *src; }
__attribute__((noinline)) void g2(struct S *dst, struct S *src) { *dst = *src; }
__attribute__((noinline)) void g3(long x, struct S *dst, struct S *src) { (void)x; *dst = *src; }
__attribute__((noinline)) long h(long double *src, long double *dst, long a) {
    long t = a * 3;
    *dst = *src;
    return t + a;
}
__attribute__((noinline)) void k(__int128 *src, __int128 *dst) {
    __int128 tmp = *src;      /* i128 lives directly in a slot */
    tmp += 1;
    memcpy(dst, &tmp, sizeof tmp);
}
__attribute__((noinline)) void m(struct T *dst, int n) {
    _Alignas(64) struct T local;
    for (int i = 0; i < 13; i++) local.c[i] = (char)('a' + ((i + n) % 26));
    memcpy(dst, &local, sizeof local);
}
__attribute__((noinline)) void mm(char *src, char *dst) {
    /* src∈rdi, dst∈rsi: memmove staging must swap too */
    __builtin_memmove(dst, src, 24);
}
__attribute__((noinline)) void many(unsigned char *src, unsigned char *dst, int n) {
    switch (n) {
#define C(N) case N: memcpy(dst, src, N); break;
        C(1) C(2) C(3) C(4) C(5) C(6) C(7) C(8) C(9) C(10) C(11) C(12) C(13) C(14) C(15) C(16)
        C(17) C(18) C(19) C(20) C(21) C(22) C(23) C(24) C(25) C(26) C(27) C(28) C(29) C(30) C(31) C(32)
        C(33) C(40) C(47) C(48) C(63) C(64)
#undef C
        default: break;
    }
}

static unsigned long fnv(const unsigned char *p, size_t n) {
    unsigned long h = 1469598103934665603ul;
    while (n--) { h ^= *p++; h *= 1099511628211ul; }
    return h;
}

int main(void) {
    struct S s = {{1, 2, 3, 4, 5, 6}}, d1 = {{0}}, d2 = {{0}}, d3 = {{0}}, d4 = {{0}};
    f(&s, &d1);
    g(&s, 7, &d2);
    g2(&d3, &s);
    g3(9, &d4, &s);
    printf("%ld %ld %ld %ld\n", d1.a[0] + d1.a[5], d2.a[1] + d2.a[4], d3.a[2] + d3.a[3], d4.a[0] * d4.a[5]);

    long double x = 2.5L, y = 0;
    long r = h(&x, &y, 5);
    printf("%ld %Lf\n", r, y);

    __int128 a = ((__int128)0x1234 << 64) | 0xfffffffffffffffful, b = 0;
    k(&a, &b);
    printf("%llx %llx\n", (unsigned long long)(b >> 64), (unsigned long long)b);

    struct T t;
    memset(&t, 0, sizeof t);
    m(&t, 3);
    printf("%.13s\n", t.c);

    char buf[40];
    for (int i = 0; i < 40; i++) buf[i] = (char)('A' + i);
    mm(buf, buf + 8);          /* forward overlap: dst > src -> backward copy */
    printf("%.40s\n", buf);
    mm(buf + 12, buf + 4);     /* dst < src -> forward copy */
    printf("%.40s\n", buf);

    unsigned char src[64], dst[64];
    for (int i = 0; i < 64; i++) src[i] = (unsigned char)(i * 7 + 1);
    static const int sizes[] = {1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20,
                                21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 40, 47, 48, 63, 64};
    unsigned long acc = 0;
    for (unsigned i = 0; i < sizeof sizes / sizeof sizes[0]; i++) {
        memset(dst, 0xee, sizeof dst);
        many(src, dst, sizes[i]);
        if (memcmp(src, dst, (size_t)sizes[i]) != 0) {
            printf("size %d MISMATCH\n", sizes[i]);
            return 1;
        }
        if (sizes[i] < 64 && dst[sizes[i]] != 0xee) {
            printf("size %d OVERRUN\n", sizes[i]);
            return 1;
        }
        acc = acc * 31 + fnv(dst, sizeof dst);
    }
    printf("%lx\n", acc);
    return 0;
}
