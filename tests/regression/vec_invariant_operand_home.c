/* A value read at one place is not a value read once.
 *
 * A vectorized byte classifier holds its constants in registers in front of
 * the loop -- the bias of each range test (`c - 59` for `c >= 59 && c <= 78`)
 * among them -- and reads each at one instruction, in every iteration.  The
 * SSE emitter has a form for a commutative op one of whose operands dies at
 * that instruction: it computes the result into the dying operand's register
 * and saves the copy.  It took "used at one place" for "dies here", and when
 * the sum had no register of its own it added the loaded bytes INTO the
 * hoisted bias:
 *
 *         movdqa .LCVEC_5(%rip), %xmm8        in front of the loop
 *     .LBB2:
 *         ...
 *         paddb %xmm4, %xmm8                   %xmm8 = bias + bytes
 *         movdqu %xmm8, %xmm1
 *
 * so every 16-byte step after the first classified against the sum of the
 * step before.  `count_outside_a` below counted 42 of the 256 byte values
 * where 12 are outside the class (GCC 16.2, Clang 23.1.5, and LCCC at -O1).
 * The form now asks what the FMA emitters ask: read once, in the block that
 * defines it (`vector_dying_values`).
 *
 * Each classifier is run over buffers that take several vector steps, filled
 * with every value of its element type that sits at or next to a bound, in
 * runs and mixed, and compared with a count made one call at a time.
 */
#include <stddef.h>
#include <stdio.h>

#define NOINLINE __attribute__((noinline))

typedef signed char sc;
typedef unsigned char uc;

/* -- classes: enough tests that the vector loop runs out of registers ----- */

static inline int class_a(sc c) {
    return c == 122 || c >= 64 || c < 47 || (c >= 59 && c <= 78);
}

static inline int class_b(uc c) {
    return c == 200 || c >= 220 || c < 9 || (c >= 59 && c <= 78) || (c >= 100 && c <= 131);
}

static inline int class_c(short c) {
    return c == 1220 || c >= 6400 || c < -470 || (c >= 590 && c <= 780) || (c >= -90 && c <= 13);
}

static inline int class_d(int c) {
    return c == 122000 || c >= 640000 || c < -47 || (c >= 59 && c <= 78) || (c >= 1000 && c <= 70000);
}

static inline int class_e(sc c) {
    return (c >= -100 && c <= -90) || (c >= -50 && c <= -41) || (c >= 3 && c <= 30) || (c >= 59 && c <= 78) ||
           c == 100 || c == -128;
}

/* -- the loops the vectorizer takes -------------------------------------- */

NOINLINE size_t count_outside_a(const sc *s, size_t n) {
    size_t k = 0;
    for (size_t i = 0; i < n; i++) {
        k += (size_t)!class_a(s[i]);
    }
    return k;
}

NOINLINE size_t count_inside_a(const sc *s, size_t n) {
    size_t k = 0;
    for (size_t i = 0; i < n; i++) {
        k += (size_t)class_a(s[i]);
    }
    return k;
}

NOINLINE size_t count_outside_b(const uc *s, size_t n) {
    size_t k = 0;
    for (size_t i = 0; i < n; i++) {
        k += (size_t)!class_b(s[i]);
    }
    return k;
}

NOINLINE size_t count_inside_c(const short *s, size_t n) {
    size_t k = 0;
    for (size_t i = 0; i < n; i++) {
        k += (size_t)class_c(s[i]);
    }
    return k;
}

NOINLINE size_t count_outside_d(const int *s, size_t n) {
    size_t k = 0;
    for (size_t i = 0; i < n; i++) {
        k += (size_t)!class_d(s[i]);
    }
    return k;
}

NOINLINE size_t count_if_e(const sc *s, size_t n) {
    size_t k = 0;
    for (size_t i = 0; i < n; i++) {
        if (class_e(s[i])) {
            k++;
        }
    }
    return k;
}

/* -- the same counts, one call a value: nothing here is vectorized ------- */

NOINLINE int in_a(int c) { return class_a((sc)c); }
NOINLINE int in_b(int c) { return class_b((uc)c); }
NOINLINE int in_c(int c) { return class_c((short)c); }
NOINLINE int in_d(int c) { return class_d(c); }
NOINLINE int in_e(int c) { return class_e((sc)c); }

/* -- driver -------------------------------------------------------------- */

#define N 83

static unsigned long long state = 0x9e3779b97f4a7c15ull;

static unsigned next(void) {
    state ^= state << 13;
    state ^= state >> 7;
    state ^= state << 17;
    return (unsigned)(state >> 11);
}

static unsigned long long sum = 0xcbf29ce484222325ull;
static int failures;

static void check(const char *what, int fill, size_t got, size_t want) {
    sum = (sum ^ got) * 0x100000001b3ull;
    if (got != want) {
        if (failures < 8) {
            printf("%s, fill %d: %zu, expected %zu\n", what, fill, got, want);
        }
        failures++;
    }
}

int main(void) {
    static sc a[N];
    static uc b[N];
    static short c[N];
    static int d[N];
    /* Bounds of the classes and their neighbours, for the wide types. */
    static const int wide[] = {-32768, -471, -470, -469, -91, -90, -48, -47, -46, 0,     12,     13,     14,
                               58,     59,   60,   78,   79,  589, 590, 780, 781, 999,   1000,   1219,   1220,
                               1221,   6399, 6400, 32767, 69999, 70000, 70001, 121999, 122000, 639999, 640000};
    const int nwide = (int)(sizeof wide / sizeof wide[0]);

    for (int fill = -128; fill < 256 + 64; fill++) {
        /* Up to 255: one value throughout, so that every vector step has to
         * give the first one's answer.  Above: mixed. */
        size_t in[5] = {0, 0, 0, 0, 0};
        for (int i = 0; i < N; i++) {
            int byte = fill < 256 ? fill : (int)(next() % 256);
            int word = fill < 256 ? wide[(fill + 128) % nwide] : wide[next() % (unsigned)nwide] + (int)(next() % 3) - 1;
            a[i] = (sc)byte;
            b[i] = (uc)byte;
            c[i] = (short)word;
            d[i] = word;
            in[0] += (size_t)in_a(a[i]);
            in[1] += (size_t)in_b(b[i]);
            in[2] += (size_t)in_c(c[i]);
            in[3] += (size_t)in_d(d[i]);
            in[4] += (size_t)in_e(a[i]);
        }
        /* Every length from a few steps down to none: the vector body, the
         * scalar remainder, and each alone. */
        size_t n = fill % 7 == 0 ? (size_t)((fill + 128) % (N + 1)) : N;
        if (n != N) {
            in[0] = in[1] = in[2] = in[3] = in[4] = 0;
            for (size_t i = 0; i < n; i++) {
                in[0] += (size_t)in_a(a[i]);
                in[1] += (size_t)in_b(b[i]);
                in[2] += (size_t)in_c(c[i]);
                in[3] += (size_t)in_d(d[i]);
                in[4] += (size_t)in_e(a[i]);
            }
        }
        check("count_outside_a", fill, count_outside_a(a, n), n - in[0]);
        check("count_inside_a", fill, count_inside_a(a, n), in[0]);
        check("count_outside_b", fill, count_outside_b(b, n), n - in[1]);
        check("count_inside_c", fill, count_inside_c(c, n), in[2]);
        check("count_outside_d", fill, count_outside_d(d, n), n - in[3]);
        check("count_if_e", fill, count_if_e(a, n), in[4]);
    }
    printf("classes %016llx, failures %d\n", sum, failures);
    return failures != 0;
}
