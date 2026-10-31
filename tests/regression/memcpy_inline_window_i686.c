/* i686 straight-line memcpy (SSE <= 256 B, scalar <= 64 B; -Os 32 / 8 B):
 * every size straddling a window or tail-width boundary, every endpoint
 * kind the expander distinguishes (frame object, pointer in a register or
 * spilled, by-value parameter home, global), checked byte for byte with
 * guard bytes on both sides of the destination. Wrappers re-run it with
 * -mno-sse and at -Os. */
#include <stdio.h>
#include <string.h>

#define NI __attribute__((noinline))
static int fails;

#define SIZES(X) X(1) X(2) X(3) X(4) X(5) X(7) X(8) X(9) X(15) X(16) X(17) \
    X(31) X(32) X(33) X(63) X(64) X(65) X(127) X(128) X(129) X(255) X(256) X(257)

#define DEFS(n)                                                                 \
    struct s##n { unsigned char b[n]; };                                        \
    struct g##n { unsigned char lo[8]; struct s##n v; unsigned char hi[8]; };   \
    static struct s##n gsrc##n;                                                 \
    NI void ptr_copy##n(struct s##n *d, const struct s##n *s) { memcpy(d, s, n); } \
    NI void assign##n(struct s##n *d, const struct s##n *s) { *d = *s; }        \
    NI void global_copy##n(struct s##n *d) { *d = gsrc##n; }                    \
    NI unsigned byval##n(struct s##n v, int k) { struct s##n t = v; return t.b[k]; } \
    NI unsigned frame##n(unsigned seed) {                                       \
        struct g##n a, b;                                                       \
        for (int i = 0; i < n; i++) a.v.b[i] = (unsigned char)(seed + i * 7);   \
        memset(&b, 0xa5, sizeof b);                                             \
        memcpy(&b.v, &a.v, n);                                                  \
        unsigned bad = 0;                                                       \
        for (int i = 0; i < 8; i++) bad |= (b.lo[i] != 0xa5) | (b.hi[i] != 0xa5); \
        for (int i = 0; i < n; i++) bad |= b.v.b[i] != (unsigned char)(seed + i * 7); \
        return bad;                                                             \
    }                                                                           \
    static void check##n(void) {                                                \
        struct g##n src, dst;                                                   \
        for (int i = 0; i < n; i++) src.v.b[i] = gsrc##n.b[i] = (unsigned char)(i * 13 + n); \
        void (*fns[3])(struct s##n *, const struct s##n *) = {ptr_copy##n, assign##n, 0}; \
        for (int f = 0; f < 3; f++) {                                           \
            memset(&dst, 0x5a, sizeof dst);                                     \
            if (fns[f]) fns[f](&dst.v, &src.v); else global_copy##n(&dst.v);    \
            int bad = 0;                                                        \
            for (int i = 0; i < 8; i++) bad |= (dst.lo[i] != 0x5a) | (dst.hi[i] != 0x5a); \
            bad |= memcmp(&dst.v, &src.v, n) != 0;                              \
            if (bad) { printf("FAIL size %d kind %d\n", n, f); fails++; }       \
        }                                                                       \
        for (int k = 0; k < n; k += (n > 16 ? n / 5 : 1))                       \
            if (byval##n(src.v, k) != src.v.b[k]) { printf("FAIL byval %d/%d\n", n, k); fails++; } \
        if (byval##n(src.v, n - 1) != src.v.b[n - 1]) { printf("FAIL byval %d end\n", n); fails++; } \
        if (frame##n(n * 3u)) { printf("FAIL frame %d\n", n); fails++; }        \
    }
SIZES(DEFS)

int main(void) {
#define RUN(n) check##n();
    SIZES(RUN)
    printf("%s %d\n", fails ? "FAILED" : "ok", fails);
    return fails != 0;
}
