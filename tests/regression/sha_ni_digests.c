/* SHA-NI end-to-end test: SHA-1 and SHA-256 block functions written with the
 * seven `_mm_sha*_epu32` / `_mm_sha1rnds4_epu32` intrinsics, checked against
 * the NIST digests.  The program exits 0 only when every digest matches.
 *
 * Each intrinsic is also printed on fixed operands, so the GCC 16.2 binary
 * (the differential oracle) must agree op by op, not only on the final hash.
 *
 * Build flags: -O2 -msha -mssse3 -msse4.1 (SHA-NI needs SSE2; SSSE3 supplies
 * `_mm_shuffle_epi8`/`_mm_alignr_epi8`, SSE4.1 supplies `_mm_blend_epi16`).
 * The host must run SHA-NI for the native binary; the GCC oracle needs it too.
 */
#include <immintrin.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

static const uint32_t K256[64] = {
    0x428a2f98U, 0x71374491U, 0xb5c0fbcfU, 0xe9b5dba5U, 0x3956c25bU, 0x59f111f1U, 0x923f82a4U, 0xab1c5ed5U,
    0xd807aa98U, 0x12835b01U, 0x243185beU, 0x550c7dc3U, 0x72be5d74U, 0x80deb1feU, 0x9bdc06a7U, 0xc19bf174U,
    0xe49b69c1U, 0xefbe4786U, 0x0fc19dc6U, 0x240ca1ccU, 0x2de92c6fU, 0x4a7484aaU, 0x5cb0a9dcU, 0x76f988daU,
    0x983e5152U, 0xa831c66dU, 0xb00327c8U, 0xbf597fc7U, 0xc6e00bf3U, 0xd5a79147U, 0x06ca6351U, 0x14292967U,
    0x27b70a85U, 0x2e1b2138U, 0x4d2c6dfcU, 0x53380d13U, 0x650a7354U, 0x766a0abbU, 0x81c2c92eU, 0x92722c85U,
    0xa2bfe8a1U, 0xa81a664bU, 0xc24b8b70U, 0xc76c51a3U, 0xd192e819U, 0xd6990624U, 0xf40e3585U, 0x106aa070U,
    0x19a4c116U, 0x1e376c08U, 0x2748774cU, 0x34b0bcb5U, 0x391c0cb3U, 0x4ed8aa4aU, 0x5b9cca4fU, 0x682e6ff3U,
    0x748f82eeU, 0x78a5636fU, 0x84c87814U, 0x8cc70208U, 0x90befffaU, 0xa4506cebU, 0xbef9a3f7U, 0xc67178f2U,
};

/* One SHA-1 four-round group; the immediate picks the round function and
 * constant (0..3), so it must be a literal in each arm. */
static __m128i sha1_group(__m128i abcd, __m128i e, int f)
{
    switch (f) {
    case 0: return _mm_sha1rnds4_epu32(abcd, e, 0);
    case 1: return _mm_sha1rnds4_epu32(abcd, e, 1);
    case 2: return _mm_sha1rnds4_epu32(abcd, e, 2);
    default: return _mm_sha1rnds4_epu32(abcd, e, 3);
    }
}

/* FIPS 180-4 SHA-1 compression of one 64-byte block, lane convention of the
 * SHA extensions: lane 3 holds the first word (A, W0), lane 0 the last. */
static void sha1_block(uint32_t state[5], const uint8_t *p)
{
    const __m128i mask = _mm_set_epi64x(0x0001020304050607LL, 0x08090a0b0c0d0e0fLL);
    __m128i abcd = _mm_shuffle_epi32(_mm_loadu_si128((const __m128i *)state), 0x1B);
    __m128i e_init = _mm_set_epi32((int)state[4], 0, 0, 0);
    __m128i abcd_save = abcd;
    __m128i m[4];
    __m128i s[21];
    __m128i e;

    for (int i = 0; i < 4; i++)
        m[i] = _mm_shuffle_epi8(_mm_loadu_si128((const __m128i *)(p + 16 * i)), mask);

    s[0] = abcd;
    e = _mm_add_epi32(e_init, m[0]);
    s[1] = sha1_group(s[0], e, 0);
    for (int g = 1; g < 20; g++) {
        if (g >= 4) {
            /* W[4g..4g+3] = msg2(msg1(W[4g-16..], W[4g-12..]) ^ W[4g-8..], W[4g-4..]) */
            __m128i x = _mm_xor_si128(_mm_sha1msg1_epu32(m[g & 3], m[(g + 1) & 3]),
                                      m[(g + 2) & 3]);
            m[g & 3] = _mm_sha1msg2_epu32(x, m[(g + 3) & 3]);
        }
        e = _mm_sha1nexte_epu32(s[g - 1], m[g & 3]);
        s[g + 1] = sha1_group(s[g], e, g / 5);
    }
    e = _mm_sha1nexte_epu32(s[19], e_init);
    abcd = _mm_add_epi32(s[20], abcd_save);
    abcd = _mm_shuffle_epi32(abcd, 0x1B);

    uint32_t e_lanes[4];
    _mm_storeu_si128((__m128i *)state, abcd);
    _mm_storeu_si128((__m128i *)e_lanes, e);
    state[4] = e_lanes[3];
}

/* FIPS 180-4 SHA-256 compression of one 64-byte block, using the two-round
 * SHA256RNDS2 state layout (ABEF in STATE0, CDGH in STATE1). */
static void sha256_block(uint32_t state[8], const uint8_t *p)
{
    const __m128i mask = _mm_set_epi64x(0x0c0d0e0f08090a0bLL, 0x0405060700010203LL);
    __m128i tmp = _mm_loadu_si128((const __m128i *)&state[0]);
    __m128i state1 = _mm_loadu_si128((const __m128i *)&state[4]);
    tmp = _mm_shuffle_epi32(tmp, 0xB1);
    state1 = _mm_shuffle_epi32(state1, 0x1B);
    __m128i state0 = _mm_alignr_epi8(tmp, state1, 8);
    state1 = _mm_blend_epi16(state1, tmp, 0xF0);
    __m128i abef_save = state0;
    __m128i cdgh_save = state1;
    __m128i m[4];

    for (int i = 0; i < 4; i++)
        m[i] = _mm_shuffle_epi8(_mm_loadu_si128((const __m128i *)(p + 16 * i)), mask);

    for (int g = 0; g < 16; g++) {
        if (g >= 4) {
            /* W[4g..] = msg2(msg1(W[4g-16..], W[4g-12..]) + alignr(W[4g-4..], W[4g-8..], 4), W[4g-4..]) */
            __m128i t = _mm_sha256msg1_epu32(m[g & 3], m[(g + 1) & 3]);
            t = _mm_add_epi32(t, _mm_alignr_epi8(m[(g + 3) & 3], m[(g + 2) & 3], 4));
            m[g & 3] = _mm_sha256msg2_epu32(t, m[(g + 3) & 3]);
        }
        __m128i wk = _mm_add_epi32(m[g & 3], _mm_loadu_si128((const __m128i *)&K256[4 * g]));
        state1 = _mm_sha256rnds2_epu32(state1, state0, wk);
        wk = _mm_shuffle_epi32(wk, 0x0E);
        state0 = _mm_sha256rnds2_epu32(state0, state1, wk);
    }
    state0 = _mm_add_epi32(state0, abef_save);
    state1 = _mm_add_epi32(state1, cdgh_save);

    tmp = _mm_shuffle_epi32(state0, 0x1B);
    state1 = _mm_shuffle_epi32(state1, 0xB1);
    state0 = _mm_blend_epi16(tmp, state1, 0xF0);
    state1 = _mm_alignr_epi8(state1, tmp, 8);
    _mm_storeu_si128((__m128i *)&state[0], state0);
    _mm_storeu_si128((__m128i *)&state[4], state1);
}

/* Pads `msg` (length n) into `buf` per FIPS 180-4: 0x80, zeros, and the
 * 64-bit big-endian bit length.  Returns the padded length (a multiple of 64);
 * `buf` must hold n + 128 bytes. */
static size_t sha_pad(const uint8_t *msg, size_t n, uint8_t *buf)
{
    memcpy(buf, msg, n);
    buf[n] = 0x80;
    size_t len = n + 1;
    while (len % 64 != 56)
        buf[len++] = 0;
    uint64_t bits = (uint64_t)n * 8;
    for (int i = 7; i >= 0; i--)
        buf[len++] = (uint8_t)(bits >> (8 * i));
    return len;
}

static void sha1_hash(const uint8_t *msg, size_t n, uint32_t out[5])
{
    static uint8_t buf[1 << 21];
    size_t len = sha_pad(msg, n, buf);
    uint32_t st[5] = {0x67452301U, 0xEFCDAB89U, 0x98BADCFEU, 0x10325476U, 0xC3D2E1F0U};
    for (size_t off = 0; off < len; off += 64)
        sha1_block(st, buf + off);
    memcpy(out, st, sizeof st);
}

static void sha256_hash(const uint8_t *msg, size_t n, uint32_t out[8])
{
    static uint8_t buf[1 << 21];
    size_t len = sha_pad(msg, n, buf);
    uint32_t st[8] = {0x6a09e667U, 0xbb67ae85U, 0x3c6ef372U, 0xa54ff53aU,
                      0x510e527fU, 0x9b05688cU, 0x1f83d9abU, 0x5be0cd19U};
    for (size_t off = 0; off < len; off += 64)
        sha256_block(st, buf + off);
    memcpy(out, st, sizeof st);
}

static int check_words(const char *name, const uint32_t *got, const uint32_t *want, int n)
{
    int ok = memcmp(got, want, (size_t)n * sizeof *want) == 0;
    printf("%s %s\n", name, ok ? "ok" : "MISMATCH");
    if (!ok) {
        for (int i = 0; i < n; i++)
            printf("  word %d: got %08x want %08x\n", i, got[i], want[i]);
    }
    return ok;
}

static void print_words(const char *name, const uint32_t *w, int n)
{
    printf("%s", name);
    for (int i = 0; i < n; i++)
        printf(" %08x", w[i]);
    printf("\n");
}

static void print_vec(const char *name, __m128i v)
{
    uint32_t w[4];
    _mm_storeu_si128((__m128i *)w, v);
    print_words(name, w, 4);
}

/* Each intrinsic on fixed operands, printed for the differential oracle. */
static void print_ops(void)
{
    const __m128i a = _mm_set_epi32(0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476);
    const __m128i b = _mm_set_epi32(0xc3d2e1f0, 0x0f1e2d3c, 0x4b5a6978, 0x87796a5b);
    const __m128i c = _mm_set_epi32(0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5);
    print_vec("sha1nexte", _mm_sha1nexte_epu32(a, b));
    print_vec("sha1msg1", _mm_sha1msg1_epu32(a, b));
    print_vec("sha1msg2", _mm_sha1msg2_epu32(a, b));
    print_vec("sha1rnds4_0", _mm_sha1rnds4_epu32(a, b, 0));
    print_vec("sha1rnds4_3", _mm_sha1rnds4_epu32(a, b, 3));
    print_vec("sha256msg1", _mm_sha256msg1_epu32(a, b));
    print_vec("sha256msg2", _mm_sha256msg2_epu32(a, b));
    print_vec("sha256rnds2", _mm_sha256rnds2_epu32(a, b, c));
}

int main(void)
{
    static const char msg_abc[] = "abc";
    static const char msg_56[] = "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
    static const uint32_t sha1_abc[5] = {0xa9993e36U, 0x4706816aU, 0xba3e2571U, 0x7850c26cU, 0x9cd0d89dU};
    static const uint32_t sha1_56[5] = {0x84983e44U, 0x1c3bd26eU, 0xbaae4aa1U, 0xf95129e5U, 0xe54670f1U};
    static const uint32_t sha1_1m[5] = {0x34aa973cU, 0xd4c4daa4U, 0xf61eeb2bU, 0xdbad2731U, 0x6534016fU};
    static const uint32_t sha256_abc[8] = {0xba7816bfU, 0x8f01cfeaU, 0x414140deU, 0x5dae2223U,
                                           0xb00361a3U, 0x96177a9cU, 0xb410ff61U, 0xf20015adU};
    static const uint32_t sha256_56[8] = {0x248d6a61U, 0xd20638b8U, 0xe5c02693U, 0x0c3e6039U,
                                          0xa33ce459U, 0x64ff2167U, 0xf6ecedd4U, 0x19db06c1U};
    static const uint32_t sha256_1m[8] = {0xcdc76e5cU, 0x9914fb92U, 0x81a1c7e2U, 0x84d73e67U,
                                          0xf1809a48U, 0xa497200eU, 0x046d39ccU, 0xc7112cd0U};
    uint32_t got[8];
    int failures = 0;

    print_ops();

    sha1_hash((const uint8_t *)msg_abc, strlen(msg_abc), got);
    failures += !check_words("sha1(abc)", got, sha1_abc, 5);
    sha1_hash((const uint8_t *)msg_56, strlen(msg_56), got);
    failures += !check_words("sha1(56)", got, sha1_56, 5);

    sha256_hash((const uint8_t *)msg_abc, strlen(msg_abc), got);
    failures += !check_words("sha256(abc)", got, sha256_abc, 8);
    sha256_hash((const uint8_t *)msg_56, strlen(msg_56), got);
    failures += !check_words("sha256(56)", got, sha256_56, 8);

    /* One million 'a': 15625 blocks, exercising the chaining across calls. */
    static uint8_t million[1000000];
    memset(million, 'a', sizeof million);
    sha1_hash(million, sizeof million, got);
    failures += !check_words("sha1(1m a)", got, sha1_1m, 5);
    sha256_hash(million, sizeof million, got);
    failures += !check_words("sha256(1m a)", got, sha256_1m, 8);

    return failures ? 1 : 0;
}
