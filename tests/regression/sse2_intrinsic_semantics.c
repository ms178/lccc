#include <emmintrin.h>
#include <stdint.h>
int main(void) {
    unsigned char a[16], z[16] = {0};
    for (int i = 0; i < 16; ++i) a[i] = (unsigned char)i;
    uint64_t sad[2];
    uint16_t sub[8];
    __m128i va = _mm_loadu_si128((const __m128i *)a);
    __m128i vz = _mm_loadu_si128((const __m128i *)z);
    _mm_storeu_si128((__m128i *)sad, _mm_sad_epu8(va, vz));
    _mm_storeu_si128((__m128i *)sub,
                     _mm_subs_epu16(_mm_set1_epi16(3), _mm_set1_epi16(5)));
    return !(sad[0] == 28 && sad[1] == 92 && sub[0] == 0 && sub[7] == 0);
}
