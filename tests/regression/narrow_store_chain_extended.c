#include <stdint.h>
static __attribute__((noinline)) uint32_t ucalc(uint32_t a, uint32_t b) {
    uint32_t x = ((a * UINT32_C(1664525)) + b) ^ (a << 13);
    return (x & UINT32_C(0x7fffffff)) | UINT32_C(0x80000000);
}
static __attribute__((noinline)) int32_t scalc(int32_t a, int32_t b) {
    /* Inputs keep every signed operation in range. */
    int32_t x = (a * 17 + b) ^ (a << 3);
    return (x & 0x3fffffff) - 1234;
}
int main(void) {
    for (uint32_t i = 0; i < 10000; ++i) {
        uint32_t a = i * 31u + 7u, b = i * 13u + 11u;
        uint32_t ref = ((a * UINT32_C(1664525) + b) ^ (a << 13));
        ref = (ref & UINT32_C(0x7fffffff)) | UINT32_C(0x80000000);
        if (ucalc(a, b) != ref) return 1;
        int32_t sa = (int32_t)(i % 1000), sb = (int32_t)(i % 97);
        int32_t sref = ((sa * 17 + sb) ^ (sa << 3));
        sref = (sref & 0x3fffffff) - 1234;
        if (scalc(sa, sb) != sref) return 2;
    }
    return 0;
}
