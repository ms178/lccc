/* Memory loads that define a 256-bit local (`_mm256_loadu_si256`,
 * `_mm256_load_si256`, `_mm256_zextsi128_si256`) are promoted to SSA values.
 * The promoted value must keep the bytes the load read, even when the source
 * memory is overwritten afterwards, and every read of the local must see that
 * copy. The checks below cover a load followed by a store to its source, an
 * aligned load, a zero extension, a dead load, a loop-carried load, a load
 * feeding a copy chain, and a load consumed by a byte shuffle. Each result is
 * printed, so the output is compared with GCC 16.2 lane by lane.
 *
 * The program needs AVX2 at run time; on a host without it the GCC reference
 * prints the same skip line, so the two outputs still compare equal. */
#include <immintrin.h>
#include <stdio.h>

static long long src[8] __attribute__((aligned(32))) = {1, 2, 3, 4, 5, 6, 7, 8};
static long long aligned_src[4] __attribute__((aligned(32))) = {10, 20, 30, 40};
static unsigned char shuf_mask[32] __attribute__((aligned(32)));
static unsigned char bytes[32] __attribute__((aligned(32)));

static void dump(const char *tag, __m256i v) {
    long long lane[4];
    _mm256_storeu_si256((__m256i *)lane, v);
    printf("%s %lld %lld %lld %lld\n", tag, lane[0], lane[1], lane[2], lane[3]);
}

/* A load, a permute, and an add. The permute reads the loaded local twice. */
__attribute__((noinline)) static __m256i permute_add(const long long *p) {
    __m256i a = _mm256_loadu_si256((const __m256i *)p);
    __m256i b = _mm256_permute4x64_epi64(a, 0x1B);
    return _mm256_add_epi64(a, b);
}

/* The source changes after the load. The loaded local must keep the old
 * bytes: the load is a copy, not a view of the memory. */
__attribute__((noinline)) static __m256i load_then_overwrite(long long *p) {
    __m256i a = _mm256_loadu_si256((const __m256i *)p);
    p[0] = 99;
    p[3] = -7;
    return _mm256_add_epi64(a, a);
}

/* Aligned load (vmovdqa) blended with a constant. */
__attribute__((noinline)) static __m256i aligned_blend(const long long *p) {
    __m256i a = _mm256_load_si256((const __m256i *)p);
    return _mm256_blend_epi32(a, _mm256_set1_epi64x(5), 0xF0);
}

/* A 128-bit load widened by a zero extension, then a cast back. The upper
 * half of the widened value must be zero. */
__attribute__((noinline)) static long long zext_cast(const long long *p) {
    __m128i lo = _mm_loadu_si128((const __m128i *)p);
    __m256i wide = _mm256_zextsi128_si256(lo);
    __m128i back = _mm256_castsi256_si128(wide);
    return _mm_cvtsi128_si64(back) + _mm256_extract_epi64(wide, 2) +
           _mm256_extract_epi64(wide, 3);
}

/* A loaded local that nothing reads. The load must have no visible effect. */
__attribute__((noinline)) static int dead_load(const long long *p) {
    __m256i unused = _mm256_loadu_si256((const __m256i *)p);
    (void)unused;
    return (int)p[1];
}

/* A loaded mask feeding a byte shuffle. The shuffle reads the mask as a
 * register operand. */
__attribute__((noinline)) static __m256i shuffle_with_loaded_mask(const unsigned char *b) {
    __m256i data = _mm256_loadu_si256((const __m256i *)b);
    __m256i mask = _mm256_loadu_si256((const __m256i *)shuf_mask);
    return _mm256_shuffle_epi8(data, mask);
}

/* A loop-carried accumulator fed by a fresh load on every iteration. */
__attribute__((noinline)) static __m256i loop_accumulate(const long long *p, int n) {
    __m256i acc = _mm256_setzero_si256();
    for (int i = 0; i < n; i++) {
        __m256i v = _mm256_loadu_si256((const __m256i *)(p + (i & 3)));
        acc = _mm256_add_epi64(acc, _mm256_permute4x64_epi64(v, 0x4E));
    }
    return acc;
}

/* A copy chain: the loaded value is stored into a second local, and the
 * second local is read by the arithmetic. */
__attribute__((noinline)) static __m256i copy_chain(const long long *p) {
    __m256i a = _mm256_loadu_si256((const __m256i *)p);
    __m256i c = a;
    return _mm256_add_epi64(c, _mm256_set1_epi64x(3));
}

int main(void) {
    __builtin_cpu_init();
    if (!__builtin_cpu_supports("avx2")) {
        printf("SKIP host lacks AVX2\n");
        return 0;
    }
    for (int i = 0; i < 32; i++) {
        shuf_mask[i] = (unsigned char)(31 - i);
        bytes[i] = (unsigned char)(i * 3 + 1);
    }
    dump("permute_add", permute_add(src));
    dump("load_then_overwrite", load_then_overwrite(src));
    printf("src_after %lld %lld %lld %lld\n", src[0], src[1], src[2], src[3]);
    dump("aligned_blend", aligned_blend(aligned_src));
    printf("zext_cast %lld\n", zext_cast(src));
    printf("dead_load %d\n", dead_load(src));
    dump("shuffle", shuffle_with_loaded_mask(bytes));
    dump("loop", loop_accumulate(src, 9));
    dump("copy_chain", copy_chain(src));
    return 0;
}
