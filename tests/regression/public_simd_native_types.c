/* Public intrinsic types must be GNU vectors, not look-alike records. */
#include <immintrin.h>

#define CONTRACT(TYPE, ELEMENT, BYTES, ALIGN) \
    typedef ELEMENT reference_##TYPE __attribute__((vector_size(BYTES))); \
    _Static_assert(sizeof(TYPE) == BYTES, #TYPE " size"); \
    _Static_assert(_Alignof(TYPE) == ALIGN, #TYPE " alignment"); \
    _Static_assert(__builtin_types_compatible_p(TYPE, reference_##TYPE), #TYPE " vector type")
CONTRACT(__m128, float, 16, 16);
CONTRACT(__m128d, double, 16, 16);
CONTRACT(__m128i, long long, 16, 16);
CONTRACT(__m128i_u, long long, 16, 1);
CONTRACT(__m128d_u, double, 16, 1);
CONTRACT(__m128_u, float, 16, 1);
CONTRACT(__m256, float, 32, 32);
CONTRACT(__m256d, double, 32, 32);
CONTRACT(__m256i, long long, 32, 32);
CONTRACT(__m256_u, float, 32, 1);
CONTRACT(__m256d_u, double, 32, 1);
CONTRACT(__m256i_u, long long, 32, 1);
CONTRACT(__m512, float, 64, 32);
CONTRACT(__m512d, double, 64, 32);
CONTRACT(__m512i, long long, 64, 32);
CONTRACT(__m512i_u, long long, 64, 1);
CONTRACT(__m512d_u, double, 64, 1);
CONTRACT(__m512_u, float, 64, 1);
#undef CONTRACT

__attribute__((noinline)) static __m256 sum(__m256 a, __m256 b)
{
    return a + b;
}

int main(void)
{
    __m128 a = {1, 2, 3, 4};
    __m128d b = {2, 5};
    __m128i c = {0x123456789abcdefLL, -1};
    __m256 x = {1, 2, 3, 4, 5, 6, 7, 8};
    __m256 y = {8, 7, 6, 5, 4, 3, 2, 1};
    __m256 (*volatile indirect)(__m256, __m256) = sum;
    __m256 z = indirect(x, y);
    __m128 array[2] = {a, a + a};
    if (array[0][3] != 4 || array[1][2] != 6)
        return 3;
    __m128i bits = (__m128i)a;
    __m128 roundtrip = (__m128)bits;
    if (roundtrip[2] != 3 || (b + b)[1] != 10 || (c ^ c)[0] != 0)
        return 1;
    for (int i = 0; i < 8; ++i)
        if (z[i] != 9)
            return 2;
    return 0;
}
