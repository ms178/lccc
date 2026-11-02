/* AUD-VABI-2: the AVX register convention for 32- and 64-byte vectors,
 * exercised across a compiler boundary in both directions.
 *
 * SysV AMD64 psABI 3.2.3: a vector is SSE + one SSEUP per further eightbyte.
 * With the AVX (AVX-512F) ABI level on, a 32-byte (64-byte) vector -- or a
 * struct/union whose only member is one -- is ONE %ymm (%zmm) register as an
 * argument and the whole value returns in %ymm0 (%zmm0).  Three rules hang
 * off that classification and each has its own function here:
 *
 *   * argument/return in one register           add8 / add4d / addi / add16,
 *                                               s32 / s64 / u32 (wrapped)
 *   * SSE overflow is transactional: the 9th    spill  (eight %ymm consumed,
 *     vector goes to memory at ITS alignment,           ninth on the stack
 *     the following scalar still takes a GPR            at 32, `k` in %rsi)
 *   * mixed scalar / vector register walk       mixed
 *   * an UNNAMED vector of a variadic call is   var / var2 (named count 1:
 *     MEMORY (psABI: the register save area has         every vector in the
 *     no room for the upper lanes); %al counts          overflow area, %al 0
 *     only the SSE registers actually used              respectively 2)
 *   * the struct that is NOT one register:      s32b (two 16-byte members:
 *     SSE+SSEUP+SSE+SSEUP is MEMORY), s40 (40 bytes), s96 (MEMORY, 64-aligned
 *     stack slot and sret return)
 *   * RVO / hidden-pointer shapes               rvo / rvo2 (returning another
 *                                               call's %ymm0 straight through)
 *   * function-pointer call                     viafp
 *   * global vector state, by-pointer           global / consume / ptr
 *
 * Without the AVX ABI level (plain -O2, -mno-avx) all of these are the memory
 * convention; both configurations are compared against gcc/gcc by the gate.
 */
#include <stdarg.h>
#include <stdio.h>

typedef float v8f __attribute__((vector_size(32)));
typedef double v4d __attribute__((vector_size(32)));
typedef int v8i __attribute__((vector_size(32)));
typedef float v16f __attribute__((vector_size(64)));
typedef float v4f __attribute__((vector_size(16)));
struct S32 { v8f v; };
struct S32b { v4f a, b; };
struct S40 { v8f v; int i; };
struct S64 { v16f v; };
struct S96 { v8f a; v16f b; };
union U32 { v8f f; v8i i; v4d d; };
typedef v8f (*fp8)(v8f, v8f);

v8f c_add8(v8f a, v8f b);
v4d c_add4d(v4d a, v4d b);
v8i c_addi(v8i a, v8i b);
v16f c_add16(v16f a, v16f b);
struct S32 c_s32(struct S32 a, struct S32 b);
struct S32b c_s32b(struct S32b a, struct S32b b);
struct S40 c_s40(struct S40 a, struct S40 b);
struct S64 c_s64(struct S64 a, struct S64 b);
struct S96 c_s96(struct S96 a, struct S96 b);
union U32 c_u32(union U32 a, union U32 b);
v8f c_mixed(int i, double d, v8f a, float f, v4f q, v8f b, long l, v16f w, v8f c);
v8f c_spill(v8f a, v8f b, v8f c, v8f d, v8f e, v8f f, v8f g, v8f h, v8f i, v8f j,
            int k, v4d z);
v8f c_var(int n, ...);
double c_var2(int n, ...);
v8f c_viafp(fp8 f, v8f a, v8f b);
v8f c_rvo(v8f a, v8f b);
v8f c_rvo2(v8f a, v8f b, int sel);
v8f c_global(void);
void c_consume(v8f a);
v8f c_ptr(const v8f *p, v8f b);
v8f c_struct_then_vec(struct S40 s, v8f v);

#ifdef CALLEE_ONLY
v8f c_add8(v8f a, v8f b) { return a + b; }
v4d c_add4d(v4d a, v4d b) { return a * b - a; }
v8i c_addi(v8i a, v8i b) { return a ^ b; }
v16f c_add16(v16f a, v16f b) { return a - b; }
struct S32 c_s32(struct S32 a, struct S32 b) {
    struct S32 r = {a.v * b.v};
    return r;
}
struct S32b c_s32b(struct S32b a, struct S32b b) {
    struct S32b r = {a.a + b.b, a.b - b.a};
    return r;
}
struct S40 c_s40(struct S40 a, struct S40 b) {
    struct S40 r = {a.v + b.v, a.i * b.i};
    return r;
}
struct S64 c_s64(struct S64 a, struct S64 b) {
    struct S64 r = {a.v + 2 * b.v};
    return r;
}
struct S96 c_s96(struct S96 a, struct S96 b) {
    struct S96 r = {a.a - b.a, a.b + b.b};
    return r;
}
union U32 c_u32(union U32 a, union U32 b) {
    union U32 r;
    r.i = a.i + b.i;
    return r;
}
v8f c_mixed(int i, double d, v8f a, float f, v4f q, v8f b, long l, v16f w, v8f c) {
    v8f r = a + b + c;
    r[0] += (float)i;
    r[1] += (float)d;
    r[2] += f;
    r[3] += q[1];
    r[4] += (float)l;
    r[5] += w[15];
    return r;
}
v8f c_spill(v8f a, v8f b, v8f c, v8f d, v8f e, v8f f, v8f g, v8f h, v8f i, v8f j,
            int k, v4d z) {
    v8f r = a + b + c + d + e + f + g + h + i + j;
    r[7] += (float)k;
    r[6] += (float)z[3];
    return r;
}
v8f c_var(int n, ...) {
    va_list ap;
    va_start(ap, n);
    v8f r = {0};
    for (int i = 0; i < n; i++) {
        v8f x = va_arg(ap, v8f);
        r += x;
    }
    va_end(ap);
    return r;
}
double c_var2(int n, ...) {
    va_list ap;
    va_start(ap, n);
    double r = 0;
    for (int i = 0; i < n; i++) {
        if (i & 1) {
            v8f x = va_arg(ap, v8f);
            r += x[i];
        } else {
            r += va_arg(ap, double);
        }
    }
    va_end(ap);
    return r;
}
v8f c_viafp(fp8 f, v8f a, v8f b) { return f(a, b); }
v8f c_rvo(v8f a, v8f b) { return c_add8(a, b); }
v8f c_rvo2(v8f a, v8f b, int sel) {
    if (sel) return c_add8(b, a);
    return c_add8(a, a);
}
static v8f g_v = {1, 2, 3, 4, 5, 6, 7, 8};
v8f c_global(void) { return g_v; }
void c_consume(v8f a) { g_v += a; }
v8f c_ptr(const v8f *p, v8f b) { return *p + b; }
v8f c_struct_then_vec(struct S40 s, v8f v) { return s.v * v + (float)s.i; }

#else /* caller TU */
static double acc;
static void ck(const char *n, const float *p, int k) {
    double s = 0;
    for (int i = 0; i < k; i++) s += p[i] * (i + 1);
    printf("%s %.3f\n", n, s);
    acc += s;
}
#define CK(n, x) ck(n, (const float *)&(x), (int)(sizeof(x) / 4))

int main(void) {
    v8f a = {1, 2, 3, 4, 5, 6, 7, 8}, b = {.5f, .25f, .125f, 1, 2, 3, 4, 5};
    v4d da = {1.5, 2.5, 3.5, 4.5}, db = {2, 3, 4, 5};
    v8i ia = {1, 2, 3, 4, 5, 6, 7, 8}, ib = {8, 7, 6, 5, 4, 3, 2, 1};
    v16f wa, wb;
    for (int i = 0; i < 16; i++) {
        wa[i] = i * 1.5f;
        wb[i] = 100 - i;
    }
    v4f q = {9, 8, 7, 6};
    v8f r;
    r = c_add8(a, b);
    CK("add8", r);
    v4d rd = c_add4d(da, db);
    printf("add4d %g %g %g %g\n", rd[0], rd[1], rd[2], rd[3]);
    v8i ri = c_addi(ia, ib);
    printf("addi %d %d %d %d\n", ri[0], ri[3], ri[5], ri[7]);
    v16f rw = c_add16(wa, wb);
    CK("add16", rw);
    struct S32 s1 = {a}, s2 = {b};
    struct S32 rs = c_s32(s1, s2);
    CK("s32", rs);
    struct S32b t1 = {q, q + 1}, t2 = {q * 2, q - 3};
    struct S32b rt = c_s32b(t1, t2);
    CK("s32b", rt);
    struct S40 u1 = {a, 3}, u2 = {b, 5};
    struct S40 ru = c_s40(u1, u2);
    CK("s40", ru.v);
    printf("s40i %d\n", ru.i);
    struct S64 w1 = {wa}, w2 = {wb};
    struct S64 rw6 = c_s64(w1, w2);
    CK("s64", rw6);
    struct S96 x1 = {a, wa}, x2 = {b, wb};
    struct S96 rx = c_s96(x1, x2);
    CK("s96a", rx.a);
    CK("s96b", rx.b);
    union U32 ua, ub;
    ua.i = ia;
    ub.i = ib;
    union U32 ruu = c_u32(ua, ub);
    printf("u32 %d %d\n", ruu.i[0], ruu.i[7]);
    r = c_mixed(7, 2.5, a, 1.25f, q, b, 1000L, wa, a);
    CK("mixed", r);
    r = c_spill(a, b, a, b, a, b, a, b, a, b, 42, da);
    CK("spill", r);
    r = c_var(3, a, b, a);
    CK("var", r);
    printf("var2 %g\n", c_var2(4, 1.5, a, 2.5, b));
    r = c_viafp(c_add8, a, b);
    CK("viafp", r);
    r = c_rvo(a, b);
    CK("rvo", r);
    r = c_rvo2(a, b, 1);
    CK("rvo2", r);
    r = c_global();
    CK("global", r);
    c_consume(b);
    r = c_global();
    CK("global2", r);
    r = c_ptr(&a, b);
    CK("ptr", r);
    r = c_struct_then_vec(u1, b);
    CK("bsv", r);
    printf("acc %.3f\n", acc);
    return 0;
}
#endif
