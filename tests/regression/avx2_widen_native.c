/* Every AVX2 widening API and GNU native-vector builtin, including a source
 * ending exactly at a guard page: the 256-bit result must not cause a 32-byte
 * input load. Exhaust all 16-bit input patterns with lane-distinct payloads. */
#define _GNU_SOURCE
#include <immintrin.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>

typedef char v16qi __attribute__((vector_size(16)));
typedef short v8hi __attribute__((vector_size(16)));
typedef int v4si __attribute__((vector_size(16)));
typedef short v16hi __attribute__((vector_size(32)));
typedef int v8si __attribute__((vector_size(32)));
typedef long long v4di __attribute__((vector_size(32)));

#define WIDEN(NAME, RAW, INPUT, OUTPUT) \
__attribute__((noinline)) static void public_##NAME(const unsigned char *p, unsigned char *out) \
{ \
    __m128i input = _mm_loadu_si128((const __m128i_u *)p); \
    __m256i result = NAME(input); \
    _mm256_storeu_si256((__m256i_u *)out, result); \
} \
__attribute__((noinline)) static void native_##NAME(const unsigned char *p, unsigned char *out) \
{ \
    INPUT input; \
    memcpy(&input, p, 16); \
    OUTPUT result = RAW(input); \
    memcpy(out, &result, 32); \
}
WIDEN(_mm256_cvtepi8_epi16, __builtin_ia32_pmovsxbw256, v16qi, v16hi)
WIDEN(_mm256_cvtepi8_epi32, __builtin_ia32_pmovsxbd256, v16qi, v8si)
WIDEN(_mm256_cvtepi8_epi64, __builtin_ia32_pmovsxbq256, v16qi, v4di)
WIDEN(_mm256_cvtepi16_epi32, __builtin_ia32_pmovsxwd256, v8hi, v8si)
WIDEN(_mm256_cvtepi16_epi64, __builtin_ia32_pmovsxwq256, v8hi, v4di)
WIDEN(_mm256_cvtepi32_epi64, __builtin_ia32_pmovsxdq256, v4si, v4di)
WIDEN(_mm256_cvtepu8_epi16, __builtin_ia32_pmovzxbw256, v16qi, v16hi)
WIDEN(_mm256_cvtepu8_epi32, __builtin_ia32_pmovzxbd256, v16qi, v8si)
WIDEN(_mm256_cvtepu8_epi64, __builtin_ia32_pmovzxbq256, v16qi, v4di)
WIDEN(_mm256_cvtepu16_epi32, __builtin_ia32_pmovzxwd256, v8hi, v8si)
WIDEN(_mm256_cvtepu16_epi64, __builtin_ia32_pmovzxwq256, v8hi, v4di)
WIDEN(_mm256_cvtepu32_epi64, __builtin_ia32_pmovzxdq256, v4si, v4di)
#undef WIDEN

typedef void (*operation)(const unsigned char *, unsigned char *);
#define ROW(NAME, FROM, TO, SIGNED) {public_##NAME, native_##NAME, FROM, TO, SIGNED}
static const struct { operation public_op, native_op; unsigned from, to, sign; } cases[] = {
    ROW(_mm256_cvtepi8_epi16, 8, 16, 1),
    ROW(_mm256_cvtepi8_epi32, 8, 32, 1),
    ROW(_mm256_cvtepi8_epi64, 8, 64, 1),
    ROW(_mm256_cvtepi16_epi32, 16, 32, 1),
    ROW(_mm256_cvtepi16_epi64, 16, 64, 1),
    ROW(_mm256_cvtepi32_epi64, 32, 64, 1),
    ROW(_mm256_cvtepu8_epi16, 8, 16, 0),
    ROW(_mm256_cvtepu8_epi32, 8, 32, 0),
    ROW(_mm256_cvtepu8_epi64, 8, 64, 0),
    ROW(_mm256_cvtepu16_epi32, 16, 32, 0),
    ROW(_mm256_cvtepu16_epi64, 16, 64, 0),
    ROW(_mm256_cvtepu32_epi64, 32, 64, 0),
};
#undef ROW

int main(void)
{
    long page = sysconf(_SC_PAGESIZE);
    if (page < 16) return 1;
    unsigned char *mapping = mmap(0, (unsigned long)page * 2, PROT_READ | PROT_WRITE,
                                  MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (mapping == MAP_FAILED) return 2;
    if (mprotect(mapping + page, (unsigned long)page, PROT_NONE)) return 3;
    unsigned char *input = mapping + page - 16;
    for (unsigned seed = 0; seed < 65536; ++seed) {
        for (unsigned byte = 0; byte < 16; ++byte)
            input[byte] = (unsigned char)((seed >> ((byte & 1) * 8)) ^ (byte * 37));
        for (unsigned c = 0; c < sizeof(cases) / sizeof(cases[0]); ++c) {
            unsigned from = cases[c].from, to = cases[c].to;
            unsigned char expected[32], public_result[32], native_result[32];
            for (unsigned lane = 0; lane < 256 / to; ++lane) {
                unsigned long long value = 0;
                for (unsigned j = 0; j < from / 8; ++j)
                    value |= (unsigned long long)input[lane * (from / 8) + j] << (j * 8);
                if (cases[c].sign && (value & (1ull << (from - 1))))
                    value |= ~((1ull << from) - 1);
                for (unsigned j = 0; j < to / 8; ++j)
                    expected[lane * (to / 8) + j] = (unsigned char)(value >> (j * 8));
            }
            cases[c].public_op(input, public_result);
            cases[c].native_op(input, native_result);
            if (memcmp(expected, public_result, 32) || memcmp(expected, native_result, 32)) {
                printf("AVX2 widening mismatch: case %u seed %u\n", c, seed);
                return 4;
            }
        }
    }
    if (munmap(mapping, (unsigned long)page * 2)) return 5;
    puts("AVX2 public/native widening: all input patterns and guard page passed");
    return 0;
}
