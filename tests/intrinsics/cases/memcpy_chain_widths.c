// FLAGS: -mavx2
/* memcpy(tmp, src); consume tmp with a whole-vector op.
 *
 * `forward_memcpy_chains` re-points the consumer at `src` and deletes the
 * copy, which is only legal when the consumer reads the whole vector at that
 * operand position. Widths are the trap: a 32-byte copy feeding a 16-byte
 * consumer position (or the reverse) must NOT be forwarded, nor may an op
 * that writes through its arguments. Every case here is run against GCC, so
 * a wrong forward shows up as different numbers rather than as a crash.
 */
#include <immintrin.h>
#include <string.h>
#include <stdio.h>

static void p128(const char *t, __m128 v) {
    float o[4];
    _mm_storeu_ps(o, v);
    printf("%s %g %g %g %g\n", t, o[0], o[1], o[2], o[3]);
}
static void p256(const char *t, __m256 v) {
    float o[8];
    _mm256_storeu_ps(o, v);
    printf("%s %g %g %g %g %g %g %g %g\n", t, o[0], o[1], o[2], o[3],
           o[4], o[5], o[6], o[7]);
}

/* Fill with a value pattern that is not symmetric, so a lane swap or a
 * half-the-width read is visible. */
static void seed(float *d, int n, float base) {
    for (int i = 0; i < n; ++i) d[i] = base + (float)i * 1.5f;
}

/* 16-byte copy feeding a 16-byte consumer: forwardable. */
__attribute__((noinline)) static __m128 f128(const float *in) {
    float src[4], tmp[4];
    seed(src, 4, 1.0f);
    memcpy(tmp, src, 16);
    __m128 a = _mm_loadu_ps(tmp);
    __m128 b = _mm_loadu_ps(in);
    return _mm_add_ps(a, b);
}

/* 32-byte copy feeding a 32-byte consumer: forwardable. */
__attribute__((noinline)) static __m256 f256(const float *in) {
    float src[8], tmp[8];
    seed(src, 8, 2.0f);
    memcpy(tmp, src, 32);
    __m256 a = _mm256_loadu_ps(tmp);
    __m256 b = _mm256_loadu_ps(in);
    return _mm256_add_ps(a, b);
}

/* 32-byte copy, but the consumer reads only the low 16 bytes through a
 * 128-bit op: the widths disagree, so forwarding must be refused. */
__attribute__((noinline)) static __m128 f256_copy_128_read(const float *in) {
    float src[8], tmp[8];
    seed(src, 8, 3.0f);
    memcpy(tmp, src, 32);
    __m128 a = _mm_loadu_ps(tmp);
    __m128 b = _mm_loadu_ps(in);
    return _mm_mul_ps(a, b);
}

/* Chained copies: src -> tmp -> tmp2, then a 32-byte consumer. Only the
 * last hop may be forwarded per pass iteration. */
__attribute__((noinline)) static __m256 f256_chain2(const float *in) {
    float src[8], tmp[8], tmp2[8];
    seed(src, 8, 4.0f);
    memcpy(tmp, src, 32);
    memcpy(tmp2, tmp, 32);
    __m256 a = _mm256_loadu_ps(tmp2);
    __m256 b = _mm256_loadu_ps(in);
    return _mm256_sub_ps(a, b);
}

/* The copy is read TWICE, so it has more than the two uses the pass
 * requires: forwarding must be refused. */
__attribute__((noinline)) static __m256 f256_two_reads(const float *in) {
    float src[8], tmp[8];
    seed(src, 8, 5.0f);
    memcpy(tmp, src, 32);
    __m256 a = _mm256_loadu_ps(tmp);
    __m256 b = _mm256_loadu_ps(tmp);
    __m256 c = _mm256_loadu_ps(in);
    return _mm256_add_ps(_mm256_mul_ps(a, b), c);
}

/* A store sits between the copy and the consumer, clobbering the source:
 * forwarding across it would read the new value instead of the old. */
__attribute__((noinline)) static __m256 f256_store_between(const float *in) {
    float src[8], tmp[8];
    seed(src, 8, 6.0f);
    memcpy(tmp, src, 32);
    __m256 keep = _mm256_loadu_ps(in);
    _mm256_storeu_ps(src, keep); /* clobbers src after the copy */
    __m256 a = _mm256_loadu_ps(tmp);
    return _mm256_add_ps(a, _mm256_set1_ps(0.5f));
}

int main(void) {
    float in[8];
    seed(in, 8, 0.25f);

    p128("f128", f128(in));
    p256("f256", f256(in));
    p128("f256_copy_128_read", f256_copy_128_read(in));
    p256("f256_chain2", f256_chain2(in));
    p256("f256_two_reads", f256_two_reads(in));
    p256("f256_store_between", f256_store_between(in));
    return 0;
}
