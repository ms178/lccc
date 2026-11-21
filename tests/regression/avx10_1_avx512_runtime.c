/* AVX10.1 runtime contract: code compiled with -mavx10.1 must run correctly on
 * a host with the AVX-512 subsets it uses. Every lane of every result is
 * checked against a scalar oracle computed in this program, so a wrong lane or
 * a wrong reduction is a nonzero exit, independent of any other compiler.
 *
 * Scope is deliberate: this fixture uses only the 512-bit and 256-bit
 * arithmetic, logic, load/store and reduction intrinsics lccc implements. The
 * AVX-512 mask-register family (`__mmask*` compares, mask-predicated ops, mask
 * loads/stores, kand/kmov) is NOT implemented and is documented as the open
 * AVX10 item; a program that calls it fails at compile time with an
 * implicit-declaration error, never with a silent miscompile.
 *
 * The program self-skips (exit 0, one SKIP line) when the host lacks AVX-512
 * F, BW, VL or DQ, so the differential harness runs everywhere. The macro half
 * of the contract is avx10_1_macros.c. */
#include <immintrin.h>
#include <stdio.h>

#define N 64

static int failures;

static void check(const char *what, int ok)
{
    if (!ok) {
        printf("MISMATCH %s\n", what);
        failures++;
    }
}

static void test_epi32_512(void)
{
    int a[N], b[N], got[N];
    for (int i = 0; i < N; i++) {
        a[i] = i * 7 - 3;
        b[i] = (i ^ 0x55) - 40;
    }
    for (int i = 0; i < N; i += 16) {
        __m512i va = _mm512_loadu_si512((const void *)&a[i]);
        __m512i vb = _mm512_loadu_si512((const void *)&b[i]);
        _mm512_storeu_si512((void *)&got[i], _mm512_add_epi32(va, vb));
    }
    for (int i = 0; i < N; i++)
        check("add_epi32 lane", got[i] == a[i] + b[i]);

    /* Multiply low 32 bits, and a horizontal reduction over one vector. */
    __m512i m = _mm512_mullo_epi32(_mm512_loadu_si512((const void *)a),
                                   _mm512_loadu_si512((const void *)b));
    _mm512_storeu_si512((void *)got, m);
    for (int i = 0; i < 16; i++)
        check("mullo_epi32 lane", got[i] == a[i] * b[i]);
    int sum_want = 0;
    for (int i = 0; i < 16; i++)
        sum_want += a[i];
    check("reduce_add_epi32", _mm512_reduce_add_epi32(
              _mm512_loadu_si512((const void *)a)) == sum_want);
    printf("epi32 ok\n");
}

static void test_bitwise_min_max(void)
{
    unsigned char a[64], b[64], out[64];
    for (int i = 0; i < 64; i++) {
        a[i] = (unsigned char)(i * 13 + 5);
        b[i] = (unsigned char)(255 - i * 3);
    }
    __m512i va = _mm512_loadu_si512((const void *)a);
    __m512i vb = _mm512_loadu_si512((const void *)b);
    _mm512_storeu_si512((void *)out, _mm512_add_epi8(va, vb));
    for (int i = 0; i < 64; i++)
        check("add_epi8 lane", out[i] == (unsigned char)(a[i] + b[i]));
    _mm512_storeu_si512((void *)out, _mm512_max_epu8(va, vb));
    for (int i = 0; i < 64; i++)
        check("max_epu8 lane", out[i] == (a[i] > b[i] ? a[i] : b[i]));
    _mm512_storeu_si512((void *)out, _mm512_xor_si512(va, vb));
    for (int i = 0; i < 64; i++)
        check("xor lane", out[i] == (unsigned char)(a[i] ^ b[i]));
    _mm512_storeu_si512((void *)out, _mm512_and_si512(va, vb));
    for (int i = 0; i < 64; i++)
        check("and lane", out[i] == (unsigned char)(a[i] & b[i]));
    printf("bitwise/minmax ok\n");
}

static void test_epi64_and_256(void)
{
    long long q[8], s[8], out[8];
    for (int i = 0; i < 8; i++) {
        q[i] = 0x100000000LL * (i + 1);
        s[i] = -(long long)i * 3;
    }
    __m512i r = _mm512_add_epi64(_mm512_loadu_si512((const void *)q),
                                 _mm512_loadu_si512((const void *)s));
    _mm512_storeu_si512((void *)out, r);
    for (int i = 0; i < 8; i++)
        check("add_epi64 lane", out[i] == q[i] + s[i]);

    long long x[4] = {1, -2, 3000000000LL, -4000000000LL};
    long long y[4] = {10, 20, -30, 40};
    __m256i v = _mm256_add_epi64(_mm256_loadu_si256((const __m256i *)x),
                                 _mm256_loadu_si256((const __m256i *)y));
    _mm256_storeu_si256((__m256i *)out, v);
    for (int i = 0; i < 4; i++)
        check("256 add_epi64 lane", out[i] == x[i] + y[i]);
    printf("epi64/256 ok\n");
}

int main(void)
{
    __builtin_cpu_init();
    if (!__builtin_cpu_supports("avx512f") || !__builtin_cpu_supports("avx512bw") ||
        !__builtin_cpu_supports("avx512vl") || !__builtin_cpu_supports("avx512dq")) {
        printf("SKIP host lacks AVX-512 F/BW/VL/DQ\n");
        return 0;
    }
    test_epi32_512();
    test_bitwise_min_max();
    test_epi64_and_256();
    if (failures) {
        printf("FAILURES %d\n", failures);
        return 1;
    }
    printf("avx10_1 runtime ok\n");
    return 0;
}
