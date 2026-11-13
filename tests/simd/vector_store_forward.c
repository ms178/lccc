// The pointer-form store builtins take their SOURCE as an address, so a computed
// vector source is materialised into a frame temporary and read straight back.
// `vector_store_forward` deletes both accesses; this fixture is the executed
// half of `tests/regression/check_vector_store_forward.sh` (the instruction
// counts are the other half).
//
// Every shape is written twice: once through the forwarding form, once through
// a form that must keep a temporary (a value read back by a second store), so a
// rewrite that is too eager changes the observed bytes rather than the count.
#include <immintrin.h>
#include <stdint.h>
#include <stdio.h>
#include <stddef.h>

#define N 64
#define BAD 0x5A5A5A5A

static int failures;

/* 256-bit float loop: the store the vectorizer produces for
 * `for (i) q[i] = p[i] + b;`. */
void float_256(float *q, const float *p, __m256 b, long n) {
    for (long i = 0; i + 8 <= n; i += 8) {
        _mm256_storeu_ps(q + i, _mm256_add_ps(_mm256_loadu_ps(p + i), b));
    }
}

/* 128-bit integer store of a computed value. */
void int_128(long long *q, const long long *p, __m128i b) {
    _mm_storeu_si128((__m128i *)q,
                     _mm_add_epi64(_mm_loadu_si128((const __m128i *)p), b));
}

/* 256-bit float store, single shot. */
void ps_256(float *q, const float *p, __m256 b) {
    _mm256_storeu_ps(q, _mm256_add_ps(_mm256_loadu_ps(p), b));
}

/* A longer chain whose final value is stored: the temporary of the LAST step is
 * the one that must disappear, while the intermediate steps stay register-only. */
void chain_256(long long *q, const long long *p, long n) {
    const __m256i k = _mm256_set1_epi64x(3);
    const __m256i m = _mm256_set1_epi64x(0x5);
    for (long i = 0; i + 4 <= n; i += 4) {
        __m256i v = _mm256_add_epi64(
            _mm256_loadu_si256((const __m256i *)(p + i)), k);
        v = _mm256_xor_si256(v, m);
        _mm256_storeu_si256((__m256i *)(q + i), _mm256_add_epi64(v, m));
    }
}

/* The value is stored TWICE: the temporary is read by two stores, so it is not
 * a single-reader temporary and the rewrite must leave it alone (kept here so
 * the "exactly three uses" rule has a live counterexample). */
void two_stores(long long *q, long long *r, const long long *p, __m128i b) {
    __m128i v = _mm_add_epi64(_mm_loadu_si128((const __m128i *)p), b);
    _mm_storeu_si128((__m128i *)q, v);
    _mm_storeu_si128((__m128i *)r, v);
}

/* The loop shapes that caught a latent miscompile in the load fold: the
 * vectorizer emits a `VecLoad` whose OFFSET operand is a RUNTIME register (not
 * a compile-time displacement), and a fold that reads the offset as "zero"
 * loads every iteration from the loop's base address.  Word lanes and float
 * lanes are both here because they take different map families. */
#define M 256
static uint16_t w_in[M], w_out[M], w_ref[M];
static float f_in[M], f_out[M], f_ref[M];

static void k_words(void) {
    for (size_t i = 0; i < M; i++) {
        w_out[i] = (uint16_t)((w_in[i] * 5u + 11u) & 0xffffu);
    }
}

static void r_words(void) {
    for (size_t i = 0; i < M; i++) {
        volatile uint32_t v = w_in[i];
        w_ref[i] = (uint16_t)((v * 5u + 11u) & 0xffffu);
    }
}

static void k_floats(void) {
    for (size_t i = 0; i < M; i++) {
        f_out[i] = f_in[i] * 1.5f + 2.25f;
    }
}

static void r_floats(void) {
    for (size_t i = 0; i < M; i++) {
        volatile float v = f_in[i];
        f_ref[i] = v * 1.5f + 2.25f;
    }
}

static void check(int cond, const char *what) {
    if (!cond) {
        printf("FAIL: %s\n", what);
        failures++;
    }
}

int main(void) {
    float pf[N], qf[N];
    long long pi[N], qi[N], ri[N];
    for (int i = 0; i < N; i++) {
        pf[i] = (float)(i * 3 - 7) / 4.0f;
        pi[i] = (long long)(i * 2654435761u) - 1000;
        qf[i] = -1.0f;
        qi[i] = -1;
        ri[i] = -1;
    }

    __m256 bf = _mm256_set1_ps(1.5f);
    float bfa[8];
    _mm256_storeu_ps(bfa, bf);

    float_256(qf, pf, bf, N);
    for (int i = 0; i < N; i++) {
        check(qf[i] == pf[i] + bfa[i % 8], "float_256 lane");
    }

    long long b128[2] = {11, 13};
    int_128(qi, pi, _mm_loadu_si128((const __m128i *)b128));
    check(qi[0] == pi[0] + 11 && qi[1] == pi[1] + 13, "int_128 lanes");

    float qs[8];
    for (int i = 0; i < 8; i++) {
        qs[i] = -1.0f;
    }
    ps_256(qs, pf + 8, bf);
    for (int i = 0; i < 8; i++) {
        check(qs[i] == pf[8 + i] + bfa[i], "ps_256 lane");
    }

    chain_256(qi, pi, N);
    for (int i = 0; i < N; i++) {
        check(qi[i] == ((pi[i] + 3) ^ 0x5) + 0x5, "chain_256 lane");
    }

    two_stores(qi, ri, pi, _mm_loadu_si128((const __m128i *)b128));
    check(qi[0] == pi[0] + 11 && qi[1] == pi[1] + 13, "two_stores first");
    check(ri[0] == pi[0] + 11 && ri[1] == pi[1] + 13, "two_stores second");

    for (int i = 0; i < M; i++) {
        w_in[i] = (unsigned short)(i * 37 + 5);
        f_in[i] = (float)i * 0.25f - 3.0f;
        w_out[i] = w_ref[i] = 0;
        f_out[i] = f_ref[i] = -1.0f;
    }
    k_words();
    r_words();
    for (int i = 0; i < M; i++) {
        check(w_out[i] == w_ref[i], "word-map lane (indexed load offset)");
    }
    k_floats();
    r_floats();
    for (int i = 0; i < M; i++) {
        check(f_out[i] == f_ref[i], "float-map lane (indexed load offset)");
    }

    if (failures == 0) {
        printf("ok  forwarded stores: 4 shapes, %d lane checks, 2 double-store "
               "lanes\n", 4 * N);
        return 0;
    }
    return 1;
}
