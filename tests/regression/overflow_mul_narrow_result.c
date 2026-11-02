/* gcc.c-torture pr71554 and pr108789: exact 32-bit products must not need
 * an I128 overflow computation on i686.  The no-overflow cases also pin the
 * mathematical signedness of mixed operands; the large unsigned case wraps
 * in the signed I64 compute type and must still be caught by the narrow
 * result round-trip. */
#define NOINLINE __attribute__((noinline, noclone))

NOINLINE int mixed_mul(unsigned a, int b, int *r) {
    return __builtin_mul_overflow(a, b, r);
}

NOINLINE int unsigned_mul(unsigned a, unsigned b, unsigned *r) {
    return __builtin_mul_overflow(a, b, r);
}

NOINLINE int signed_mul(int a, int b, int *r) {
    return __builtin_mul_overflow(a, b, r);
}

#define CHECK(call, expected_overflow, expected_result)                      \
    do {                                                                     \
        if ((call) != (expected_overflow) || r != (expected_result))          \
            return __LINE__;                                                 \
    } while (0)

int main(void) {
    const int min = -__INT_MAX__ - 1;
    const unsigned umax = ~0U;
    {
        int r;
        CHECK(mixed_mul(2, min / 2, &r), 0, min);
        CHECK(mixed_mul(3, min / 2, &r), 1, __INT_MAX__ / 2 + 1);
        CHECK(mixed_mul(1, -1, &r), 0, -1);
        CHECK(mixed_mul(umax, -1, &r), 1, 1);
        CHECK(signed_mul(min, min, &r), 1, 0);
        CHECK(signed_mul(min, -1, &r), 1, min);
    }
    {
        unsigned r;
        CHECK(unsigned_mul(256, 256, &r), 0, 65536U);
        CHECK(unsigned_mul(65536, 65536, &r), 1, 0U);
        CHECK(unsigned_mul(0x80000000U, 2, &r), 1, 0U);
        CHECK(unsigned_mul(umax, umax, &r), 1, 1U);
    }
    return 0;
}
