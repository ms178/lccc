/* Defined signed wraps must occur before pointer-width extension. No huge
 * mapping or out-of-object base is used: every correct address is in data.
 * Compile with -fwrapv; volatile inputs and noinline keep the narrow operators. */
#include <stdint.h>
#include <stdio.h>
#define NI __attribute__((noinline))
static NI int mul2(const unsigned char *p, int32_t i) { return p[i * 2]; }
static NI int mul8(const unsigned char *p, int32_t i) { return p[i * 8]; }
static NI int twice(const unsigned char *p, int32_t i) { return p[i + i]; }
static NI int add_top(const unsigned char *p, int32_t i) { return p[i + INT32_MAX]; }
static NI int sub_top(const unsigned char *p, int32_t i) { return p[i - INT32_MAX]; }
static NI int nested(const unsigned char *p, int32_t i) { return p[(i + 1) * 2]; }
static NI int wide(const unsigned char *p, int64_t i) { return p[i * 2]; }
static NI int trunc16(const unsigned char *p, int16_t i) { return p[(int16_t)(i * 2)]; }
static volatile int32_t input32;
static volatile int64_t input64;
int main(void) {
    unsigned char data[32];
    for (unsigned j = 0; j < 32; ++j) data[j] = (unsigned char)(j + 1);
    const unsigned char *p = data + 16;
    unsigned cases = 0;
#define TEST(fn, input, offset) do { \
    input32 = (input); int result = fn(p, input32); \
    if (result != p[offset]) { printf("signed scale mismatch: %s %d\n", #fn, result); return 1; } \
    ++cases; \
} while (0)
#ifdef SCALE_SAFE_ONLY
    TEST(mul2, -1, -2);
    TEST(mul8, -1, -8);
    TEST(twice, -1, -2);
    TEST(add_top, INT32_MIN, -1);
    TEST(sub_top, INT32_MAX, 0);
    TEST(nested, -2, -2);
    TEST(trunc16, -1, -2);
    input64 = -1;
    if (wide(p, input64) != p[-2]) return 1;
    ++cases;
    const unsigned expected_cases = 8;
#else
    TEST(mul2, INT32_MIN, 0);
    TEST(mul2, INT32_MAX, -2);
    TEST(mul2, -1, -2);
    TEST(mul2, 1, 2);
    TEST(mul8, INT32_C(536870912), 0);
    TEST(mul8, INT32_MAX, -8);
    TEST(twice, INT32_MIN, 0);
    TEST(twice, INT32_MAX, -2);
    TEST(add_top, INT32_MAX, -2);
    TEST(add_top, INT32_MIN, -1);
    TEST(sub_top, INT32_MIN, 1);
    TEST(sub_top, INT32_MAX, 0);
    TEST(nested, INT32_MAX, 0);
    TEST(nested, INT32_MAX - 1, -2);
    TEST(trunc16, INT16_MIN, 0);
    input64 = INT64_MIN;
    if (wide(p, input64) != p[0]) return 1;
    ++cases;
    const unsigned expected_cases = 16;
#endif
    printf("signed scale ring: PASS %u\n", cases);
    return cases != expected_cases;
}
