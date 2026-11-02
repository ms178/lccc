/* Loop-carried store-to-load forwarding (src/passes/loop_carried_forward.rs).
 *
 * `a[i] = f(a[i-1])` must become a register recurrence (the load of a[i-1]
 * is replaced by a header phi fed by the previous iteration's stored value)
 * WITHOUT changing semantics in any of the shapes below. Every kernel is
 * self-checking against a reference computed with the transform provably
 * inapplicable (volatile / opaque pointer), and the exit code is the
 * verdict (RULES.md #5). Mutation-verified: forcing the preheader load
 * to `addr_L(1)` (off-by-one), skipping the `|stride| >= size` guard, or
 * ignoring the interleaved-field overlap test each makes at least one of
 * these kernels abort.
 *
 * Shapes:
 *   k1  canonical prefix recurrence on a global (tls_seg_access shape),
 *       int IV widened by iv_widen, volatile aliasing *read* in the body
 *   k2  descending stride (a[i] = a[i+1] ^ k), signed I64 IV
 *   k3  chained distance-2 recurrence (Fibonacci table) — two rounds
 *   k4  interleaved struct fields: write s[i].x, read s[i-1].x, and an
 *       unrelated store to s[i].y that shares the object and stride
 *   k5  same-object store at a DIFFERENT stride in the body — must NOT fire
 *       (memory result is the oracle either way)
 *   k6  zero-trip loop on an alloca: preheader load must be in-bounds-safe
 *       or not speculated; the untouched array must remain untouched
 *   k7  do-while shape with the store value being the load itself
 *       (degenerate copy a[i] = a[i-1])
 *   k8  loop with a pointer parameter (opaque root): forwarding through the
 *       same pointer is legal; a second opaque pointer store must veto it
 *   k9  byte-sized elements with |stride| == size (edge of the overlap
 *       guard) and unsigned char wrap
 */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define N 64
static uint64_t g_slots[N + 2];

__attribute__((noinline)) static uint64_t k1(unsigned p) {
    uint64_t acc = 0;
    g_slots[1] = p;
    for (int i = 2; i <= N; i++) {
        g_slots[i] = g_slots[i - 1] + p;
        acc += g_slots[i] ^ *(volatile uint64_t *)&g_slots[(i & 7) + 1];
    }
    return acc + g_slots[N];
}
__attribute__((noinline)) static uint64_t k1_ref(unsigned p) {
    volatile uint64_t *s = g_slots;
    uint64_t acc = 0;
    s[1] = p;
    for (int i = 2; i <= N; i++) {
        s[i] = s[i - 1] + p;
        acc += s[i] ^ s[(i & 7) + 1];
    }
    return acc + s[N];
}

__attribute__((noinline)) static uint64_t k2(uint64_t *a, int64_t n, uint64_t k) {
    a[n] = k;
    for (int64_t i = n - 1; i >= 0; i--)
        a[i] = (a[i + 1] * 0x9E3779B97F4A7C15ull) ^ k;
    return a[0];
}
__attribute__((noinline)) static uint64_t k2_ref(volatile uint64_t *a, int64_t n, uint64_t k) {
    a[n] = k;
    for (int64_t i = n - 1; i >= 0; i--)
        a[i] = (a[i + 1] * 0x9E3779B97F4A7C15ull) ^ k;
    return a[0];
}

__attribute__((noinline)) static uint32_t k3(uint32_t *f, long n) {
    f[0] = 1;
    f[1] = 1;
    for (long i = 2; i < n; i++)
        f[i] = f[i - 1] + f[i - 2];
    return f[n - 1];
}
__attribute__((noinline)) static uint32_t k3_ref(volatile uint32_t *f, long n) {
    f[0] = 1;
    f[1] = 1;
    for (long i = 2; i < n; i++)
        f[i] = f[i - 1] + f[i - 2];
    return f[n - 1];
}

struct pair {
    uint64_t x, y;
};
__attribute__((noinline)) static uint64_t k4(struct pair *s, long n) {
    s[0].x = 7;
    s[0].y = 3;
    for (long i = 1; i < n; i++) {
        s[i].x = s[i - 1].x * 3 + 1;
        s[i].y = s[i].x ^ (uint64_t)i;
    }
    return s[n - 1].x + s[n - 1].y;
}
__attribute__((noinline)) static uint64_t k4_ref(volatile struct pair *s, long n) {
    s[0].x = 7;
    s[0].y = 3;
    for (long i = 1; i < n; i++) {
        s[i].x = s[i - 1].x * 3 + 1;
        s[i].y = s[i].x ^ (uint64_t)i;
    }
    return s[n - 1].x + s[n - 1].y;
}

static uint64_t g_k5[2 * N + 4];
__attribute__((noinline)) static uint64_t k5(uint64_t seed) {
    g_k5[0] = seed;
    for (long i = 1; i < N; i++) {
        g_k5[i] = g_k5[i - 1] + 1;
        g_k5[2 * i] = 0xFFu; /* different stride: overlaps a[i-1] for i even */
    }
    return g_k5[N - 1] + g_k5[N - 2];
}
__attribute__((noinline)) static uint64_t k5_ref(uint64_t seed) {
    volatile uint64_t *a = g_k5;
    a[0] = seed;
    for (long i = 1; i < N; i++) {
        a[i] = a[i - 1] + 1;
        a[2 * i] = 0xFFu;
    }
    return a[N - 1] + a[N - 2];
}

__attribute__((noinline)) static uint64_t k6(long n) {
    uint64_t buf[8];
    memset(buf, 0xAB, sizeof buf);
    buf[0] = 1;
    for (long i = 1; i < n; i++)
        buf[i] = buf[i - 1] << 1;
    uint64_t h = 0;
    for (int i = 0; i < 8; i++)
        h = h * 31 + buf[i];
    return h;
}
__attribute__((noinline)) static uint64_t k6_ref(long n) {
    volatile uint64_t buf[8];
    for (int i = 0; i < 8; i++)
        buf[i] = 0xABABABABABABABABull;
    buf[0] = 1;
    for (long i = 1; i < n; i++)
        buf[i] = buf[i - 1] << 1;
    uint64_t h = 0;
    for (int i = 0; i < 8; i++)
        h = h * 31 + buf[i];
    return h;
}

__attribute__((noinline)) static uint32_t k7(uint32_t *a, long n, uint32_t v) {
    a[0] = v;
    long i = 1;
    do {
        a[i] = a[i - 1];
        i++;
    } while (i < n);
    return a[n - 1];
}

__attribute__((noinline)) static uint64_t k8(uint64_t *a, uint64_t *b, long n) {
    /* b may alias a: forwarding a[i-1] across the b[i] store is illegal. */
    for (long i = 1; i < n; i++) {
        a[i] = a[i - 1] + 5;
        b[i] = 1;
    }
    return a[n - 1];
}
__attribute__((noinline)) static uint64_t k8_ref(volatile uint64_t *a, volatile uint64_t *b, long n) {
    for (long i = 1; i < n; i++) {
        a[i] = a[i - 1] + 5;
        b[i] = 1;
    }
    return a[n - 1];
}

__attribute__((noinline)) static unsigned k9(unsigned char *c, long n) {
    c[0] = 200;
    for (long i = 1; i < n; i++)
        c[i] = (unsigned char)(c[i - 1] + 37);
    unsigned h = 0;
    for (long i = 0; i < n; i++)
        h = h * 131 + c[i];
    return h;
}
__attribute__((noinline)) static unsigned k9_ref(volatile unsigned char *c, long n) {
    c[0] = 200;
    for (long i = 1; i < n; i++)
        c[i] = (unsigned char)(c[i - 1] + 37);
    unsigned h = 0;
    for (long i = 0; i < n; i++)
        h = h * 131 + c[i];
    return h;
}

static uint64_t A[N + 2], B[N + 2];
static uint32_t F[N + 2];
static struct pair P[N + 2];
static unsigned char C[N + 2];

int main(void) {
    int fails = 0;
#define CHECK(id, got, want)                                                  \
    do {                                                                      \
        unsigned long long g_ = (unsigned long long)(got), w_ = (want);       \
        if (g_ != w_) {                                                       \
            printf("%s: got %llu want %llu\n", id, g_, w_);                   \
            fails++;                                                          \
        }                                                                     \
    } while (0)

    /* The volatile read at slot (i & 7) + 1 runs ahead of the write front
       for i < 8, so both variants must start from identical memory. */
    for (unsigned p = 1; p < 40; p += 13) {
        memset(g_slots, 0, sizeof g_slots);
        uint64_t got = k1(p);
        memset(g_slots, 0, sizeof g_slots);
        CHECK("k1", got, k1_ref(p));
    }

    memset(A, 0, sizeof A);
    memset(B, 0, sizeof B);
    CHECK("k2", k2(A, N, 0x1234567ull), k2_ref(B, N, 0x1234567ull));
    if (memcmp(A, B, sizeof A) != 0) {
        printf("k2: memory differs\n");
        fails++;
    }

    memset(F, 0, sizeof F);
    uint32_t f_got = k3(F, 40);
    uint32_t f_snapshot[N + 2];
    memcpy(f_snapshot, F, sizeof F);
    memset(F, 0, sizeof F);
    CHECK("k3", f_got, k3_ref(F, 40));
    if (memcmp(F, f_snapshot, sizeof F) != 0) {
        printf("k3: memory differs\n");
        fails++;
    }

    memset(P, 0, sizeof P);
    uint64_t p_got = k4(P, N);
    struct pair p_snapshot[N + 2];
    memcpy(p_snapshot, P, sizeof P);
    memset(P, 0, sizeof P);
    CHECK("k4", p_got, k4_ref(P, N));
    if (memcmp(P, p_snapshot, sizeof P) != 0) {
        printf("k4: memory differs\n");
        fails++;
    }

    uint64_t k5_got = k5(9);
    uint64_t k5_mem[2 * N + 4];
    memcpy(k5_mem, g_k5, sizeof g_k5);
    memset(g_k5, 0, sizeof g_k5);
    CHECK("k5", k5_got, k5_ref(9));
    if (memcmp(k5_mem, g_k5, sizeof g_k5) != 0) {
        printf("k5: memory differs\n");
        fails++;
    }

    CHECK("k6-zero-trip", k6(0), k6_ref(0));
    CHECK("k6-one-trip", k6(1), k6_ref(1));
    CHECK("k6-full", k6(8), k6_ref(8));

    memset(F, 0, sizeof F);
    CHECK("k7", k7(F, 17, 0xC0FFEEu), 0xC0FFEEu);
    for (int i = 0; i < 17; i++)
        if (F[i] != 0xC0FFEEu) {
            printf("k7: F[%d]=%u\n", i, F[i]);
            fails++;
            break;
        }

    /* k8 with b == a + 1: b[i] overwrites a[i+1] before the next iteration
       reads it (a[i] = a[i-1] + 5 must see the 1 written by b). */
    memset(A, 0, sizeof A);
    memset(B, 0, sizeof B);
    A[0] = 100;
    B[0] = 100;
    CHECK("k8-alias", k8(A, A + 1, 32), k8_ref(B, B + 1, 32));
    if (memcmp(A, B, sizeof A) != 0) {
        printf("k8: memory differs\n");
        fails++;
    }
    /* k8 with disjoint arrays. */
    memset(A, 0, sizeof A);
    memset(B, 0, sizeof B);
    A[0] = 100;
    uint64_t k8d = k8(A, B, 32);
    uint64_t A2[N + 2], B2[N + 2];
    memset(A2, 0, sizeof A2);
    memset(B2, 0, sizeof B2);
    A2[0] = 100;
    CHECK("k8-disjoint", k8d, k8_ref(A2, B2, 32));
    if (memcmp(A, A2, sizeof A) != 0 || memcmp(B, B2, sizeof B) != 0) {
        printf("k8-disjoint: memory differs\n");
        fails++;
    }

    memset(C, 0, sizeof C);
    unsigned c_got = k9(C, N);
    unsigned char c_snapshot[N + 2];
    memcpy(c_snapshot, C, sizeof C);
    memset(C, 0, sizeof C);
    CHECK("k9", c_got, k9_ref(C, N));
    if (memcmp(C, c_snapshot, sizeof C) != 0) {
        printf("k9: memory differs\n");
        fails++;
    }

    if (fails) {
        printf("loop_carried_store_load_forward: %d failure(s)\n", fails);
        fflush(stdout);
        abort();
    }
    printf("loop_carried_store_load_forward: ok\n");
    return 0;
}
