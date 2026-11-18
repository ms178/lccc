/* i686 AVX1-legal 256-bit splats and high-lane extract (-m32 -mavx).
 *
 * `vpbroadcast{b,w,d,q}` and `vextracti128` are AVX2-only. Under -mavx the i686
 * backend must replicate the scalar with VEX.128 shuffles and `vinsertf128`,
 * and extract the high lane with `vextractf128`. GCC 16.2 accepts the same
 * source with `-m32 -mavx` and emits no AVX2 instruction.
 *
 * Freestanding (no libc): the host has no 32-bit libc. The exit status is the
 * number of wrong lanes, so any mismatch against GCC 16.2 changes the status.
 * The splat scalars carry garbage in the bits above the element width, which
 * the byte and word splats must ignore. Values are loaded from volatile
 * storage so the splats are runtime splats, not folded constants. */
#include <immintrin.h>

static int bad;

static void check_b(__m256i v, unsigned char want) {
    unsigned char b[32];
    _mm256_storeu_si256((__m256i *)b, v);
    for (int i = 0; i < 32; i++)
        bad += (b[i] != want);
}

static void check_w(__m256i v, unsigned short want) {
    unsigned short w[16];
    _mm256_storeu_si256((__m256i *)w, v);
    for (int i = 0; i < 16; i++)
        bad += (w[i] != want);
}

static void check_d(__m256i v, unsigned int want) {
    unsigned int d[8];
    _mm256_storeu_si256((__m256i *)d, v);
    for (int i = 0; i < 8; i++)
        bad += (d[i] != want);
}

static void check_q(__m256i v, unsigned long long want) {
    unsigned long long q[4];
    _mm256_storeu_si256((__m256i *)q, v);
    for (int i = 0; i < 4; i++)
        bad += (q[i] != want);
}

volatile int src_b = 0x7a5a; /* only the low byte 0x5a is the element */
volatile int src_w = 0x71234; /* only the low word 0x1234 is the element */
volatile int src_d = 0x89abcdef;
volatile long long src_q = 0x0123456789abcdefLL;

/* A 32-byte array of distinct dwords, loaded as one vector. */
static unsigned int ramp[8] = {0x10, 0x21, 0x32, 0x43, 0x54, 0x65, 0x76, 0x87};

__attribute__((noinline)) static void splats(void) {
    check_b(_mm256_set1_epi8((char)src_b), 0x5a);
    check_w(_mm256_set1_epi16((short)src_w), 0x1234);
    check_d(_mm256_set1_epi32(src_d), 0x89abcdefu);
    check_q(_mm256_set1_epi64x(src_q), 0x0123456789abcdefULL);
}

__attribute__((noinline)) static void extract(void) {
    __m256i v = _mm256_loadu_si256((const __m256i *)ramp);
    /* High lane: dwords 4..7 (0x54, 0x65, 0x76, 0x87). */
    __m128i hi = _mm_castps_si128(__builtin_ia32_vextractf128_ps256(_mm256_castsi256_ps(v), 1));
    /* Low lane: dwords 0..3 (0x10, 0x21, 0x32, 0x43). */
    __m128i lo = _mm256_castsi256_si128(v);
    unsigned int h[4], l[4];
    _mm_storeu_si128((__m128i *)h, hi);
    _mm_storeu_si128((__m128i *)l, lo);
    bad += (h[0] != 0x54) + (h[1] != 0x65) + (h[2] != 0x76) + (h[3] != 0x87);
    bad += (l[0] != 0x10) + (l[1] != 0x21) + (l[2] != 0x32) + (l[3] != 0x43);
}

__attribute__((noreturn)) void _start(void) {
    splats();
    extract();
    int status = bad ? 1 : 0;
    __asm__ volatile("int $0x80" : : "a"(1), "b"(status) : "memory");
    __builtin_unreachable();
}
