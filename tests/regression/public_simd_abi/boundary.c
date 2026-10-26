/* Compile provider and caller separately, with each compiler's own headers.
 * Nine vectors overflow both ABI register files; mixed XMM/YMM arguments,
 * GP scalars, a 64-bit scalar, direct calls and volatile indirect calls must
 * preserve EVERY byte. No intrinsic or floating arithmetic can conceal an
 * ABI error by recomputing the result on the caller side. */
#include <immintrin.h>
#include <string.h>
#include <stdio.h>
#include <stdarg.h>

#ifndef BITS
#define BITS 256
#endif
#ifndef KIND
#define KIND 0
#endif
#if BITS == 128
# if KIND == 0
  typedef __m128 T;
# elif KIND == 1
  typedef __m128d T;
# else
  typedef __m128i T;
# endif
#else
# if KIND == 0
  typedef __m256 T;
# elif KIND == 1
  typedef __m256d T;
# else
  typedef __m256i T;
# endif
#endif
#define N (BITS / 8)
#define PARAMETERS int cookie, T a, T b, __m128 middle, T c, T d, T e, T f, T g, T h, T i, long long tail

#if PROVIDER
__attribute__((noinline)) T boundary(PARAMETERS)
{
    const T values[9] = {a, b, c, d, e, f, g, h, i};
    unsigned char result[N];
    for (int lane = 0; lane < N; ++lane) {
        unsigned v = (unsigned)cookie + (unsigned)((unsigned long long)tail >> (8 * (lane % 8)));
        v ^= ((const unsigned char *)&middle)[lane % 16];
        for (int arg = 0; arg < 9; ++arg)
            v = v * 33u + ((const unsigned char *)&values[arg])[(lane + arg * 3) % N];
        result[lane] = (unsigned char)v;
    }
    T out;
    memcpy(&out, result, N);
    return out;
}
__attribute__((noinline)) T varboundary(T first, int cookie, ...)
{
    va_list ap;
    va_start(ap, cookie);
    T second = va_arg(ap, T);
    unsigned tag = va_arg(ap, unsigned);
    va_end(ap);
    T out;
    for (int lane = 0; lane < N; ++lane)
        ((unsigned char *)&out)[lane] = ((const unsigned char *)&first)[lane]
            ^ ((const unsigned char *)&second)[lane] ^ (unsigned char)(cookie + tag + lane);
    return out;
}
#else
extern T boundary(PARAMETERS);
extern T varboundary(T first, int cookie, ...);
int main(void)
{
    unsigned char bytes[9][N], mid[16], expected[N], actual[N];
    T values[9];
    __m128 middle;
    for (int iteration = 0; iteration < 17; ++iteration) {
        for (int arg = 0; arg < 9; ++arg) {
            for (int lane = 0; lane < N; ++lane)
                bytes[arg][lane] = (unsigned char)(iteration * 97 + arg * 43 + lane * 13);
            memcpy(&values[arg], bytes[arg], N);
        }
        for (int lane = 0; lane < 16; ++lane)
            mid[lane] = (unsigned char)(lane * 19 + iteration * 7);
        memcpy(&middle, mid, 16);
        int cookie = 0x1234567 + iteration;
        long long tail = 0x123456789abcdefLL + iteration;
        for (int lane = 0; lane < N; ++lane) {
            unsigned v = (unsigned)cookie + (unsigned)((unsigned long long)tail >> (8 * (lane % 8)));
            v ^= mid[lane % 16];
            for (int arg = 0; arg < 9; ++arg)
                v = v * 33u + bytes[arg][(lane + arg * 3) % N];
            expected[lane] = (unsigned char)v;
        }
        T (*volatile indirect)(PARAMETERS) = boundary;
        for (int mode = 0; mode < 2; ++mode) {
#define ARGS cookie, values[0], values[1], middle, values[2], values[3], values[4], values[5], values[6], values[7], values[8], tail
            T out = mode ? indirect(ARGS) : boundary(ARGS);
#undef ARGS
            memcpy(actual, &out, N);
            if (memcmp(actual, expected, N)) {
                fprintf(stderr, "ABI mismatch: bits=%d kind=%d iteration=%d indirect=%d\n", BITS, KIND, iteration, mode);
                for (int byte = 0; byte < N; ++byte)
                    fprintf(stderr, "%02x/%02x%c", actual[byte], expected[byte], byte + 1 == N ? '\n' : ' ');
                return 1;
            }
        }
        T (*volatile indirect_va)(T, int, ...) = varboundary;
        for (int mode = 0; mode < 2; ++mode) {
            T out = mode ? indirect_va(values[2], cookie, values[7], 0xfedcba98u)
                         : varboundary(values[2], cookie, values[7], 0xfedcba98u);
            memcpy(actual, &out, N);
            for (int lane = 0; lane < N; ++lane) {
                unsigned char wanted = bytes[2][lane] ^ bytes[7][lane]
                    ^ (unsigned char)((unsigned)cookie + 0xfedcba98u + lane);
                if (actual[lane] != wanted) {
                    fprintf(stderr, "variadic ABI mismatch: bits=%d kind=%d iteration=%d indirect=%d lane=%d\n", BITS, KIND, iteration, mode, lane);
                    return 2;
                }
            }
        }
    }
    puts("public native SIMD ABI: PASS");
    return 0;
}
#endif
