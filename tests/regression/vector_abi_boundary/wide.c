/* AUD-VABI-2 reproducer: 32-byte VECTOR arguments and returns across a
 * compiler boundary.
 *
 * The SysV AMD64 psABI classifies a vector as SSE + one SSEUP per further
 * eightbyte, so with AVX enabled a 32-byte vector travels in ONE %ymm
 * ("the upper part of the same register").  lccc has no SSEUP notion and its
 * struct rule stops at 16 bytes, so a 32-byte vector falls back to the
 * memory/sret convention instead -- invisible inside an all-lccc program,
 * fatal the moment a gcc-compiled library takes or returns one:
 *
 *   measured at the commit that added this file (see the gate):
 *     gcc/gcc                      -> r=42 ...        (the reference)
 *     lccc/lccc                    -> r=42 ...        (self-consistent)
 *     lccc caller + gcc callee     -> r=0 0 0 ...     (silent garbage)
 *     gcc caller + lccc callee     -> SIGSEGV
 *
 * The 16-byte case of exactly this bug is AUD-VABI-1 (fixed and gated, see the
 * sibling vector_abi.c); the 32-byte case needs a %ymm carrier the object
 * format/lowering path does not have yet, so it is tracked, measured and
 * latched rather than left to be rediscovered by a user.
 *
 * The struct-shaped control (`struct big`) is here on purpose: a 32-byte
 * STRUCT must keep the two-register / memory convention the psABI gives it
 * (SSE+SSE for `struct { double a,b,c,d; }`'s four eightbytes is not SSEUP),
 * so a fix for the vector case must not sweep it along.
 */
#include <stdio.h>

typedef int v8si __attribute__((vector_size(32)));
typedef double v4df __attribute__((vector_size(32)));

struct big {
    double a, b, c, d;
};

#ifdef CALLEE_ONLY
v8si add_v8si(v8si a, v8si b) { return a + b; }

v8si splat_v8si(int k) {
    v8si r;
    for (int i = 0; i < 8; i++) r[i] = k;
    return r;
}

v8si scale_v8si(v8si a, int k) { return a * k; }

int sum_v8si(int tag, v8si a, int k) {
    int s = tag + k;
    for (int i = 0; i < 8; i++) s += a[i];
    return s;
}

v4df add_v4df(v4df a, v4df b) { return a + b; }

struct big add_big(struct big x, struct big y) {
    struct big r;
    r.a = x.a + y.a;
    r.b = x.b + y.b;
    r.c = x.c + y.c;
    r.d = x.d + y.d;
    return r;
}

#else /* caller TU: declarations only */
v8si add_v8si(v8si a, v8si b);
v8si splat_v8si(int k);
v8si scale_v8si(v8si a, int k);
int sum_v8si(int tag, v8si a, int k);
v4df add_v4df(v4df a, v4df b);
struct big add_big(struct big x, struct big y);

static void show(const char *what, v8si v) {
    printf("%s:", what);
    for (int i = 0; i < 8; i++) printf(" %d", v[i]);
    printf("\n");
}

int main(void) {
    v8si a = splat_v8si(7), b = splat_v8si(35);
    v4df f1 = {1.5, 2.5, 3.5, 4.5}, f2 = {10.0, 20.0, 30.0, 40.0};
    struct big x = {1.0, 2.0, 3.0, 4.0}, y = {10.0, 20.0, 30.0, 40.0};

    show("add", add_v8si(a, b));
    show("scale", scale_v8si(a, 5));
    printf("sum: %d\n", sum_v8si(1000, a, 7));
    {
        v4df fr = add_v4df(f1, f2);
        printf("v4df: %.2f %.2f %.2f %.2f\n", fr[0], fr[1], fr[2], fr[3]);
    }
    {
        struct big br = add_big(x, y);
        printf("big: %.2f %.2f %.2f %.2f\n", br.a, br.b, br.c, br.d);
    }
    return 0;
}
#endif
