#include <stdint.h>
#include <stdio.h>

/* Every address is inside one object. Exercise both subscript orderings and
   both pointer-add orderings: arithmetic expressions have I32 defining values,
   even when a literal makes the older storage-type query report I64. */
static uint64_t data[257];
#define NOINLINE __attribute__((noinline))
#define INDEX(name, expr) \
    NOINLINE static uint64_t name(const uint64_t *p, int a, int b) { \
        return p[(expr)]; \
    }
INDEX(add, a + b)
INDEX(sub, a - b)
INDEX(mul, a * b)
INDEX(bitand, a & b)
INDEX(bitor, a | b)
INDEX(bitxor, a ^ b)
INDEX(quotient, a / b)
INDEX(rem_index, a % b)
NOINLINE static uint64_t add_literal(const uint64_t *p, int a) { return p[a + 1]; }
NOINLINE static uint64_t reverse(const uint64_t *p, int a) { return (a - 1)[p]; }
NOINLINE static uint64_t ptr_right(const uint64_t *p, int a) { return *(p + (a + 1)); }
NOINLINE static uint64_t ptr_left(const uint64_t *p, int a) { return *((a + 1) + p); }
NOINLINE static uint64_t ptr_sub(const uint64_t *p, int a) { return *(p - (a + 1)); }
NOINLINE static uint64_t negate(const uint64_t *p, int a) { return p[-a]; }
NOINLINE static uint64_t invert(const uint64_t *p, int a) { return p[~a]; }
NOINLINE static uint64_t select_index(const uint64_t *p, int a, int b) {
    return p[a < b ? a + 1 : b - 1];
}

int main(void) {
    for (unsigned i = 0; i < 257; ++i)
        data[i] = (uint64_t)(i + 3) * UINT64_C(0x9e3779b185ebca87);
    const uint64_t *p = data + 128;
    unsigned checks = 0;
#define CHECK(got, index) do { if ((got) != p[(long long)(index)]) return 1; ++checks; } while (0)
    for (int a = -12; a <= 12; ++a) {
        CHECK(add_literal(p, a), (long long)a + 1);
        CHECK(reverse(p, a), (long long)a - 1);
        CHECK(ptr_right(p, a), (long long)a + 1);
        CHECK(ptr_left(p, a), (long long)a + 1);
        CHECK(ptr_sub(p, a), -((long long)a + 1));
        CHECK(negate(p, a), -(long long)a);
        CHECK(invert(p, a), ~(long long)a);
        for (int b = -4; b <= 4; ++b) {
            CHECK(add(p, a, b), (long long)a + b);
            CHECK(sub(p, a, b), (long long)a - b);
            CHECK(mul(p, a, b), (long long)a * b);
            CHECK(bitand(p, a, b), (long long)a & b);
            CHECK(bitor(p, a, b), (long long)a | b);
            CHECK(bitxor(p, a, b), (long long)a ^ b);
            CHECK(select_index(p, a, b), a < b ? (long long)a + 1 : (long long)b - 1);
            if (b) {
                CHECK(quotient(p, a, b), (long long)a / b);
                CHECK(rem_index(p, a, b), (long long)a % b);
            }
        }
    }
    printf("x86_signed_index_home: PASS (%u checks)\n", checks);
    return 0;
}
