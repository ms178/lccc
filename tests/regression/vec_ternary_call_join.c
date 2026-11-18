/* Regression: a 128-bit `?:` over two call results must deliver the direct
 * arm's returned bytes to the join.
 *
 * The direct arm's result was copied into a join value whose register home is
 * an XMM register. The copy was emitted as an alias of %xmm0 (nothing written),
 * so the join read whatever that XMM register held before the call. That was
 * the `middle` argument, so the direct path returned the argument's bytes.
 * The indirect arm was correct, and -O0 was correct.
 *
 * Output must be byte-identical to GCC for every iteration and both arms.
 */
#include <emmintrin.h>
#include <stdio.h>
#include <string.h>

typedef __m128i T;

__attribute__((noinline)) T boundary(int cookie, T a, T b, __m128 middle, T c, T d,
                                     T e, T f, T g, T h, T i, long long tail)
{
    const T values[9] = {a, b, c, d, e, f, g, h, i};
    unsigned char result[16];
    for (int lane = 0; lane < 16; ++lane) {
        unsigned v = (unsigned)cookie
            + (unsigned)((unsigned long long)tail >> (8 * (lane % 8)));
        v ^= ((const unsigned char *)&middle)[lane];
        for (int arg = 0; arg < 9; ++arg)
            v = v * 33u + ((const unsigned char *)&values[arg])[(lane + arg * 3) % 16];
        result[lane] = (unsigned char)v;
    }
    T out;
    memcpy(&out, result, 16);
    return out;
}

int main(void)
{
    for (int iteration = 0; iteration < 4; ++iteration) {
        T values[9];
        for (int arg = 0; arg < 9; ++arg) {
            unsigned char bytes[16];
            for (int lane = 0; lane < 16; ++lane)
                bytes[lane] = (unsigned char)(iteration * 97 + arg * 43 + lane * 13);
            memcpy(&values[arg], bytes, 16);
        }
        unsigned char mid[16];
        for (int lane = 0; lane < 16; ++lane)
            mid[lane] = (unsigned char)(lane * 19 + iteration * 7);
        __m128 middle;
        memcpy(&middle, mid, 16);
        int cookie = 0x1234567 + iteration;
        long long tail = 0x123456789abcdefLL + iteration;
        T (*volatile indirect)(int, T, T, __m128, T, T, T, T, T, T, T, long long) = boundary;
        for (int mode = 0; mode < 2; ++mode) {
            T out = mode
                ? indirect(cookie, values[0], values[1], middle, values[2], values[3],
                           values[4], values[5], values[6], values[7], values[8], tail)
                : boundary(cookie, values[0], values[1], middle, values[2], values[3],
                           values[4], values[5], values[6], values[7], values[8], tail);
            unsigned char actual[16];
            memcpy(actual, &out, 16);
            for (int lane = 0; lane < 16; ++lane)
                printf("%02x", actual[lane]);
            printf(" iter=%d mode=%d\n", iteration, mode);
        }
    }
    return 0;
}
