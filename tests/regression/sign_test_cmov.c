#define INT_MAX __INT_MAX__
#define INT_MIN (-INT_MAX - 1)

/* Keep assembly-only -m32 coverage independent of multilib libc headers.
 * The runtime leg still requires a working libc and reports its own probe. */
extern int printf(const char *format, ...);

#if defined(__GNUC__) || defined(__clang__)
#define NOINLINE __attribute__((noinline))
#else
#define NOINLINE
#endif

/* Contract: compile this fixture with -fwrapv; its boundary matrix deliberately
 * overflows signed subtraction. Without that flag the oracle comparison would
 * exercise undefined source behavior rather than wrapped-result semantics.
 *
 * The result is compared with zero after subtraction. The `ge`/`l` condition
 * produced by TEST uses only the result's sign bit; the x86 peephole can read
 * the same sign directly from SUB's flags. */
NOINLINE int nonnegative_difference(int a, int b) {
    int difference = a - b;
    return difference < 0 ? 0 : difference;
}

NOINLINE int negative_difference(int a, int b) {
    int difference = a - b;
    return difference < 0 ? difference : 0;
}

/* Strict positivity also depends on ZF. This is a negative control: the
 * sign-only peephole must leave its TEST/condition pair intact. */
NOINLINE int positive_difference(int a, int b) {
    int difference = a - b;
    return difference > 0 ? difference : 0;
}

int main(void) {
    static const int values[] = {
        INT_MIN, INT_MIN + 1, -1, 0, 1, INT_MAX - 1, INT_MAX,
    };
    unsigned long long checksum = 0;
    for (unsigned i = 0; i < sizeof(values) / sizeof(values[0]); ++i) {
        for (unsigned j = 0; j < sizeof(values) / sizeof(values[0]); ++j) {
            int a = values[i];
            int b = values[j];
            checksum = checksum * 33 + (unsigned)nonnegative_difference(a, b);
            checksum = checksum * 33 + (unsigned)negative_difference(a, b);
            checksum = checksum * 33 + (unsigned)positive_difference(a, b);
        }
    }
    printf("%llu\n", checksum);
    return 0;
}
