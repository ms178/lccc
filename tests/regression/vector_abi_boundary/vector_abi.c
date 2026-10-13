/* AUD-VABI-1 gate input: 16-byte (and 8-byte) VECTOR arguments and returns
 * across a compiler boundary.
 *
 * The SysV AMD64 psABI classifies the FIRST eightbyte of a vector as SSE and
 * every further eightbyte as SSEUP -- "the upper half of the same register" --
 * so `<4 x i32>` travels in ONE %xmm, unlike a `{double, double}` struct whose
 * two SSE eightbytes take two registers.  lccc classified vectors with the
 * *struct* rule, which has no SSEUP notion, so:
 *
 *   * its caller put the two halves of each argument into a GP pair where gcc's
 *     callee read %xmm0/%xmm1, and read the result from %rdx:%rax where gcc's
 *     callee had written %xmm0 -- `lccc main.o + gcc ext.o` printed
 *     `16 0 65536 0` for an answer of `11 22 33 44`;
 *   * its callee read %rdi/%rsi/%rdx/%rcx where gcc's caller had passed
 *     %xmm0/%xmm1, and returned the first argument unchanged.
 *
 * Linking a program against system libraries compiled by gcc is the normal
 * case, so this file is compiled twice -- once as the CALLEE by each compiler
 * and once as the CALLER by each compiler -- and all four combinations must
 * print the same numbers.
 *
 * The struct-shaped control (`struct two_f64`) is in the file on purpose: it
 * must KEEP the two-register convention (SSE + SSE, two registers), which is
 * what makes the vector rule a special case rather than a rewrite.
 */
#include <stdio.h>

typedef int v4si __attribute__((vector_size(16)));
typedef int v2si __attribute__((vector_size(8)));
typedef long long v2di __attribute__((vector_size(16)));
typedef double v2df __attribute__((vector_size(16)));

struct two_f64 { double a, b; };
struct mixed { int n; v4si v; };

#ifdef CALLEE_ONLY
/* Every shape a boundary can take: vector-only, vector among scalars (so the
 * GP and SSE register sequences are interleaved), 8-byte and 16-byte vectors,
 * a struct embedding one, and a vector smuggled through a function pointer. */
/* Self-test hook for the gate (never compiled into a real run): with
 * -DVABI_CALLEE_MUTATION the vector callees answer with something that is NOT
 * the ABI answer, and the gate requires its own comparison to notice.  A gate
 * that diffs the wrong file passes vacuously -- this gate did exactly that
 * until review -- so the mutation is what keeps the comparison honest.
 *
 * The macro form matters: the mutation must not remove any definition, or the
 * mutated object fails to link for an unrelated reason and the check would be
 * measuring the linker instead. */
#ifdef VABI_CALLEE_MUTATION
#define VABI_ANSWER(a, b) (b)
#define VABI_BIAS(n) ((n) + 1)
#else
#define VABI_ANSWER(a, b) ((a) + (b))
#define VABI_BIAS(n) (n)
#endif

v4si add_v4si(v4si a, v4si b) { return VABI_ANSWER(a, b); }

v4si scale_v4si(v4si a, int k) { return a * VABI_BIAS(k); }

int sum_v4si(int tag, v4si a, int k) { return tag + a[0] + a[1] + a[2] + a[3] + VABI_BIAS(k); }

v4si pick_third(v4si a, v4si b, v4si c) { return VABI_ANSWER(c, b); }

v4si add_three(v4si a, v4si b, v4si c) { return a + b + c + (v4si){VABI_BIAS(0), VABI_BIAS(0), VABI_BIAS(0), VABI_BIAS(0)}; }

v2si add_v2si(v2si a, v2si b) { return VABI_ANSWER(a, b); }

v2di xor_v2di(v2di a, v2di b) { return VABI_ANSWER(a, b); }

v2df add_v2df(v2df a, v2df b) { return VABI_ANSWER(a, b); }

struct two_f64 add_two_f64(struct two_f64 x, struct two_f64 y) {
    struct two_f64 r;
    r.a = x.a + y.a;
    r.b = x.b + y.b;
    return r;
}

v4si mixed_apply(struct mixed m, v4si extra) { return m.v * m.n + extra; }

v4si (*pick_impl(int which))(v4si, v4si, v4si) { return which ? pick_third : add_three; }

#else /* caller TU: declarations only */
/* Caller half: the SAME declarations, no definitions (the gate links this
 * object against an object built with -DCALLEE_ONLY). */
v4si add_v4si(v4si a, v4si b);
v4si scale_v4si(v4si a, int k);
int sum_v4si(int tag, v4si a, int k);
v4si pick_third(v4si a, v4si b, v4si c);
v4si add_three(v4si a, v4si b, v4si c);
v2si add_v2si(v2si a, v2si b);
v2di xor_v2di(v2di a, v2di b);
v2df add_v2df(v2df a, v2df b);
struct two_f64 add_two_f64(struct two_f64 x, struct two_f64 y);
v4si mixed_apply(struct mixed m, v4si extra);
v4si (*pick_impl(int which))(v4si, v4si, v4si);


static void show(const char *what, v4si v) {
    printf("%s: %d %d %d %d\n", what, v[0], v[1], v[2], v[3]);
}

int main(void) {
    v4si a = {1, 2, 3, 4}, b = {10, 20, 30, 40}, c = {100, 200, 300, 400};
    v2si s1 = {7, 8}, s2 = {1, 2};
    v2di d1 = {0x0102030405060708LL, 0x1112131415161718LL};
    v2di d2 = {0x00ff00ff00ff00ffLL, 0x0000ffff0000ffffLL};
    v2df f1 = {1.5, 2.25}, f2 = {0.5, 0.75};
    struct two_f64 p = {1.5, 2.5}, q = {10.25, 20.5};
    struct mixed m = {3, {2, 4, 6, 8}};
    struct two_f64 pr = add_two_f64(p, q);

    show("add", add_v4si(a, b));
    show("scale", scale_v4si(a, 5));
    printf("sum: %d\n", sum_v4si(1000, a, 7));
    show("third", pick_third(a, b, c));
    show("v2si", (v4si){s1[0] + s2[0], s1[1] + s2[1], 0, 0});
    show("v2di", (v4si){xor_v2di(d1, d2)[0] & 0x7fffffff, 0, 0, 0});
    show("v2df", (v4si){(int)add_v2df(f1, f2)[0], (int)add_v2df(f1, f2)[1], 0, 0});
    printf("two_f64: %.2f %.2f\n", pr.a, pr.b);
    show("mixed", mixed_apply(m, a));
    show("fptr", pick_impl(0)(a, b, c));
    show("fptr2", pick_impl(1)(a, b, c));
    return 0;
}
#endif
