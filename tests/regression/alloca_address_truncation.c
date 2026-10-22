/* Every consumer of a LOCAL's address must see the ADDRESS.
 *
 * An alloca's stack slot is the storage the alloca denotes, not a home for
 * the value: the value of an alloca is the slot's address. The x86-64
 * register-direct narrowing cast read that slot as if it were the value's
 * data home, so `(int)(uintptr_t)&s` loaded `s` (`movl slot, %reg`) instead
 * of materializing `leaq slot` and truncating it. Concretely, at -O1/-O2/-O3
 *
 *     int f(void) { int s = 7; return (int)(unsigned long)&s; }
 *
 * returned 7 on every run, and `(int)(uintptr_t)&arr[0]` returned arr[0].
 * -O0 was correct (a different lowering), and globals were correct (a global
 * address is a symbol, not an alloca slot), which is why this survived: it
 * only shows up when the address itself is truncated to 32 bits or less.
 *
 * The oracle has to live INSIDE the function that owns the local. Comparing
 * a callee's address against a caller's variable proves nothing: the two are
 * different stack slots, so any "mismatch" is the comparison, not the
 * compiler. Each check below therefore takes the untruncated address of its
 * own local and compares it against the truncated form of that same address.
 *
 * Expected: every check reports ok and the program exits 0. GCC and Clang
 * agree (differential oracle: stdout must match byte for byte).
 */
#include <stdio.h>
#include <stdint.h>
#include <string.h>

/* ---- 1. direct integer casts of a local's address ---------------------- */

/* The reported shape: pointer -> int truncation. The second comparison is
 * what makes the shape bite: it keeps the untruncated address live in a
 * register next to the cast, which is the register pressure that steered the
 * cast onto the register-direct path in the first place. It also states the
 * defect positively: a truncated stack address is never the stored 7. */
static int c_narrow_int(void) {
    int s = 7;
    uintptr_t a = (uintptr_t)&s;
    return (int)(uintptr_t)&s == (int)a && (int)a != 7;
}

/* unsigned destination, same width (zero-extend instead of sign-extend) */
static int c_narrow_uint(void) {
    int s = 7;
    uintptr_t a = (uintptr_t)&s;
    return (unsigned)(uintptr_t)&s == (unsigned)a && (unsigned)a != 7;
}

/* sub-word destinations */
static int c_narrow_subword(void) {
    int s = 7;
    uintptr_t a = (uintptr_t)&s;
    return (short)(uintptr_t)&s == (short)a
        && (unsigned short)(uintptr_t)&s == (unsigned short)a
        && (signed char)(uintptr_t)&s == (signed char)a
        && (unsigned char)(uintptr_t)&s == (unsigned char)a;
}

/* no truncation: same-size and wide casts must keep the whole address */
static int c_same_size(void) {
    int s = 7;
    uintptr_t a = (uintptr_t)&s;
    return (unsigned long)(uintptr_t)&s == (unsigned long)a
        && (long long)(uintptr_t)&s == (long long)a
        && (long)(uintptr_t)&s == (long)a;
}

/* two chained casts: truncate then widen back (sign and zero flavours) */
static int c_chain(void) {
    int s = 7;
    uintptr_t a = (uintptr_t)&s;
    return (long)(int)(uintptr_t)&s == (long)(int)a
        && (unsigned long)(unsigned)(uintptr_t)&s == (unsigned long)(unsigned)a
        && (int)(long)(uintptr_t)&s == (int)(long)a;
}

/* ---- 2. the address reaching the cast through other routes ------------- */

/* ternary / phi selection between two live addresses */
static int c_phi(int c) {
    int s = 7;
    uintptr_t a = (uintptr_t)&s;
    int t = c ? (int)(uintptr_t)&s : (int)a;
    return t == (int)a;
}

/* loop-carried value */
static int c_loop(void) {
    int s = 7;
    uintptr_t a = (uintptr_t)&s;
    int t = 0;
    for (int i = 0; i < 3; i++) t = (int)(uintptr_t)&s;
    return t == (int)a;
}

/* arithmetic on the address, then truncate */
static int c_arith(void) {
    int s = 7;
    uintptr_t a = (uintptr_t)&s;
    return (int)((uintptr_t)&s + 1) == (int)(a + 1)
        && (int)((uintptr_t)&s >> 3) == (int)(a >> 3)
        && (int)(((uintptr_t)&s + 1) >> 3) == (int)((a + 1) >> 3)
        && (int)(~(uintptr_t)&s) == (int)~a
        && (int)((uintptr_t)&s & 0xf0UL) == (int)(a & 0xf0UL)
        && (int)((uintptr_t)&s | 0xfUL) == (int)(a | 0xfUL)
        && (int)((uintptr_t)&s ^ 0x55UL) == (int)(a ^ 0x55UL)
        && (int)((uintptr_t)&s % 64UL) == (int)(a % 64UL)
        && (int)((uintptr_t)&s / 16UL) == (int)(a / 16UL);
}

/* through memory: array, struct, union, memcpy */
static int c_memory(void) {
    int s = 7;
    uintptr_t a = (uintptr_t)&s;
    uintptr_t arr[2];
    struct { void *p; uintptr_t raw; } bx;
    union { uintptr_t raw; void *p; } u;
    uintptr_t src = a, dst = 0;
    arr[0] = (uintptr_t)&s;
    arr[1] = a;
    bx.p = &s;
    bx.raw = (uintptr_t)&s;
    u.p = &s;
    memcpy(&dst, &src, sizeof dst);
    return (int)arr[0] == (int)arr[1] && (int)arr[1] == (int)a
        && bx.p == (void *)&s && (int)bx.raw == (int)a
        && (int)u.raw == (int)a && (int)dst == (int)a;
}

/* through a noinline callee's parameter (the address is passed, the callee
 * truncates: the cast there must not read the alloca the caller owns) */
static int lo32(uintptr_t x) { return (int)x; }
static int c_arg(void) {
    int s = 7;
    uintptr_t a = (uintptr_t)&s;
    return lo32((uintptr_t)&s) == lo32(a);
}

/* escaped address: handed to an opaque callee, compared there against the
 * caller's own untruncated view of the same frame slot */
static int probe_cb(const void *p, uintptr_t want) { return (uintptr_t)p == want; }
static int c_escape(void) {
    int s = 7;
    uintptr_t a = (uintptr_t)&s;
    return probe_cb(&s, a) && (int)(uintptr_t)&s == (int)a;
}

/* ---- 3. other addressable local object shapes -------------------------- */

/* array base (the second reported shape: returned arr[0]) */
static int c_array(void) {
    int v[4];
    v[0] = 11;
    v[1] = 12;
    uintptr_t a = (uintptr_t)v;
    return (int)(uintptr_t)v == (int)a && (int)(uintptr_t)&v[0] == (int)a
        && (int)a != 11;
}

/* runtime-indexed element */
static int c_element(int i) {
    int v[8];
    uintptr_t a = (uintptr_t)&v[i];
    for (int k = 0; k < 8; k++) v[k] = k * 3;
    return (int)(uintptr_t)&v[i] == (int)a;
}

/* struct member, including one behind a padding gap */
static int c_member(void) {
    struct big {
        int a;
        char pad[24];
        long b;
    } r;
    struct { int x, y; } pts[4];
    r.a = 1;
    r.b = 2;
    pts[2].x = 5;
    pts[2].y = 6;
    uintptr_t rb = (uintptr_t)&r.b, py = (uintptr_t)&pts[2].y;
    return (long)(uintptr_t)&r.b == (long)rb
        && (int)(uintptr_t)&pts[2].y == (int)py && (int)py != 6;
}

/* locals of the other scalar types (different sizes and alignments) */
static int c_scalars(void) {
    long L = 1;
    char C = 2;
    double D = 3.0;
    short S = 4;
    uintptr_t al = (uintptr_t)&L, ac = (uintptr_t)&C, ad = (uintptr_t)&D,
              as = (uintptr_t)&S;
    return (int)(uintptr_t)&L == (int)al && (int)(uintptr_t)&C == (int)ac
        && (long)(uintptr_t)&D == (long)ad && (unsigned)(uintptr_t)&S == (unsigned)as;
}

/* vector-typed local: the shape that re-exposed this class (a 16-byte
 * alloca whose first lane is the tempting "constant" to fold) */
typedef int v4i32 __attribute__((vector_size(16)));
static v4i32 vret(v4i32 a) { return a; }
static int c_vector(void) {
    v4i32 r = vret((v4i32){3, 0, 2, 1});
    uintptr_t a = (uintptr_t)&r;
    int t = (int)(uintptr_t)&r;
    return t == (int)a && r[0] == 3 && r[1] == 0 && r[2] == 2 && r[3] == 1;
}

/* Over-aligned locals are realigned at runtime. Their address cast must use
 * that adjusted address rather than a fixed frame-slot displacement. The
 * volatile global keeps the truncated cast observable so this exercises the
 * direct-cast emitter instead of only comparing two folded copies. */
static volatile uint32_t overalign_low;
static int c_overaligned(void) {
    volatile int s __attribute__((aligned(64))) = 7;
    uintptr_t a = (uintptr_t)&s;
    overalign_low = (uint32_t)(uintptr_t)&s;
    return overalign_low == (uint32_t)a && (a & 63u) == 0 && s == 7;
}

/* native int -> double / float conversions of the address (a different
 * lowering, pinned so it cannot regress into the same fold) */
static int c_float(void) {
    int s = 7;
    uintptr_t a = (uintptr_t)&s;
    return (double)(uintptr_t)&s == (double)a && (float)(uintptr_t)&s == (float)a;
}

/* volatile local: still an alloca, still an address */
static int c_volatile(void) {
    volatile int s = 7;
    uintptr_t a = (uintptr_t)&s;
    return (int)(uintptr_t)&s == (int)a && (int)a != 7;
}

/* nested scope */
static int c_nested(void) {
    uintptr_t a;
    {
        int s = 7;
        a = (uintptr_t)&s;
        if ((int)(uintptr_t)&s != (int)a) return 0;
    }
    return 1;
}

/* ---- driver ------------------------------------------------------------ */

struct test {
    const char *name;
    int (*fn)(void);
};

static int c_phi_zero(void) { return c_phi(0); }
static int c_phi_one(void) { return c_phi(1); }
static int c_element_zero(void) { return c_element(0); }
static int c_element_three(void) { return c_element(3); }

int main(void) {
    static const struct test tests[] = {
        {"narrow_int", c_narrow_int},
        {"narrow_uint", c_narrow_uint},
        {"narrow_subword", c_narrow_subword},
        {"same_size", c_same_size},
        {"chain", c_chain},
        {"phi0", c_phi_zero},
        {"phi1", c_phi_one},
        {"loop", c_loop},
        {"arith", c_arith},
        {"memory", c_memory},
        {"arg", c_arg},
        {"escape", c_escape},
        {"array", c_array},
        {"element0", c_element_zero},
        {"element3", c_element_three},
        {"member", c_member},
        {"scalars", c_scalars},
        {"vector", c_vector},
        {"overaligned", c_overaligned},
        {"float", c_float},
        {"volatile", c_volatile},
        {"nested", c_nested},
    };
    int failures = 0;

    for (unsigned i = 0; i < sizeof tests / sizeof tests[0]; i++) {
        int ok = tests[i].fn();
        if (!ok) {
            printf("FAIL %s\n", tests[i].name);
            failures++;
        } else {
            printf("ok %s\n", tests[i].name);
        }
    }
    printf(failures ? "FAILURES: %d\n" : "OK alloca_address_truncation\n", failures);
    return failures != 0;
}
