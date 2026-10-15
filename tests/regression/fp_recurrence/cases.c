/* Exact IEEE results and sticky exceptions, not a tolerance comparison.
 * The volatile/call-based references cannot use either new optimization. */
#ifndef NO_FENV_PRAGMA
#pragma STDC FENV_ACCESS ON
#endif
#include <fenv.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#if defined(__x86_64__)
#include <xmmintrin.h>
#endif
#define NI __attribute__((noinline))
static uint64_t digest = 14695981039346656037ULL;
static void hash_word(uint64_t word) {
    for (unsigned i = 0; i < 8; ++i) {
        digest = (digest ^ (word & 255)) * 1099511628211ULL;
        word >>= 8;
    }
}

NI double twice64(double x) { return 2.0 * x; }
NI float twice32(float x) { return x * 2.0f; }
NI double twice_plus(double x, double y) { return x * 2.0 + y; }
NI double twice_minus(double x, double y) { return 2.0 * x - y; }
NI float twice_plus32(float x, float y) { return 2.0f * x + y; }
static NI double reference_plus(double x, double y) { volatile double two = 2.0; return two * x + y; }
static NI float reference_plus32(float x, float y) { volatile float two = 2.0f; return two * x + y; }
NI double scaled_fma(double x, double y, double z) { return 2.0 * x * y + z; }
static NI double reference64(double x) { volatile double two = 2.0; return two * x; }
static NI float reference32(float x) { volatile float two = 2.0f; return x * two; }
static NI double square_reference(double x) { volatile double copy = x; return copy * x; }

NI double square_loop(double c, int n) {
    double x = -0.0, sum = 0.0;
    for (int i = 0; i < n; ++i) {
        double old = x * x;
        x = old + c;
        double next = x * x;
        sum += old + next;
    }
    return sum + x;
}
static NI double reference_loop(double c, int n) {
    double x = -0.0, sum = 0.0;
    for (int i = 0; i < n; ++i) {
        double old = square_reference(x);
        x = old + c;
        double next = square_reference(x);
        sum += old + next;
    }
    return sum + x;
}
/* Dynamic environment changes forbid carrying a previously rounded square. */
NI double changing_round(double c, int n) {
    double x = 0.0, sum = 0.0;
    for (int i = 0; i < n; ++i) {
        fesetround(i & 1 ? FE_UPWARD : FE_DOWNWARD);
        double old = x * x;
        x = old + c;
        double next = x * x;
        sum += old + next;
    }
    return sum + x;
}
static NI double reference_changing(double c, int n) {
    double x = 0.0, sum = 0.0;
    for (int i = 0; i < n; ++i) {
        fesetround(i & 1 ? FE_UPWARD : FE_DOWNWARD);
        double old = square_reference(x);
        x = old + c;
        double next = square_reference(x);
        sum += old + next;
    }
    return sum + x;
}
static uint64_t bits64(double x) { uint64_t u; memcpy(&u, &x, sizeof u); return u; }
static uint32_t bits32(float x) { uint32_t u; memcpy(&u, &x, sizeof u); return u; }
static int check_double(uint64_t u) {
    double x; memcpy(&x, &u, sizeof x);
    feclearexcept(FE_ALL_EXCEPT);
    double expected = reference64(x);
    int ef = fetestexcept(FE_ALL_EXCEPT);
    feclearexcept(FE_ALL_EXCEPT);
    double actual = twice64(x);
    int af = fetestexcept(FE_ALL_EXCEPT);
    hash_word(bits64(expected)); hash_word((unsigned)ef);
    hash_word(bits64(actual)); hash_word((unsigned)af);
    if (bits64(expected) != bits64(actual) || ef != af) {
        printf("double %llx: %llx/%x != %llx/%x\n", (unsigned long long)u,
            (unsigned long long)bits64(expected), ef, (unsigned long long)bits64(actual), af);
        return 1;
    }
    for (int subtract = 0; subtract < 2; ++subtract) {
        feclearexcept(FE_ALL_EXCEPT);
        expected = reference_plus(x, subtract ? -0.125 : 0.125);
        ef = fetestexcept(FE_ALL_EXCEPT);
        feclearexcept(FE_ALL_EXCEPT);
        actual = subtract ? twice_minus(x, 0.125) : twice_plus(x, 0.125);
        af = fetestexcept(FE_ALL_EXCEPT);
        hash_word(bits64(expected)); hash_word((unsigned)ef);
        hash_word(bits64(actual)); hash_word((unsigned)af);
        if (bits64(expected) != bits64(actual) || ef != af) {
            printf("double consumer %llx/%d mismatch\n", (unsigned long long)u, subtract);
            return 1;
        }
    }
    return 0;
}
static int check_float(uint32_t u) {
    float x; memcpy(&x, &u, sizeof x);
    feclearexcept(FE_ALL_EXCEPT);
    float expected = reference32(x);
    int ef = fetestexcept(FE_ALL_EXCEPT);
    feclearexcept(FE_ALL_EXCEPT);
    float actual = twice32(x);
    int af = fetestexcept(FE_ALL_EXCEPT);
    hash_word(bits32(expected)); hash_word((unsigned)ef);
    hash_word(bits32(actual)); hash_word((unsigned)af);
    if (bits32(expected) != bits32(actual) || ef != af) {
        printf("float %x: %x/%x != %x/%x\n", u, bits32(expected), ef, bits32(actual), af);
        return 1;
    }
    feclearexcept(FE_ALL_EXCEPT);
    expected = reference_plus32(x, 0.125f);
    ef = fetestexcept(FE_ALL_EXCEPT);
    feclearexcept(FE_ALL_EXCEPT);
    actual = twice_plus32(x, 0.125f);
    af = fetestexcept(FE_ALL_EXCEPT);
    hash_word(bits32(expected)); hash_word((unsigned)ef);
    hash_word(bits32(actual)); hash_word((unsigned)af);
    if (bits32(expected) != bits32(actual) || ef != af) {
        printf("float consumer %x mismatch\n", u);
        return 1;
    }
    return 0;
}
static int check_loop(double c, int n, int changing) {
    feclearexcept(FE_ALL_EXCEPT);
    double expected = changing ? reference_changing(c, n) : reference_loop(c, n);
    int ef = fetestexcept(FE_ALL_EXCEPT);
    feclearexcept(FE_ALL_EXCEPT);
    double actual = changing ? changing_round(c, n) : square_loop(c, n);
    int af = fetestexcept(FE_ALL_EXCEPT);
    hash_word(bits64(expected)); hash_word((unsigned)ef);
    hash_word(bits64(actual)); hash_word((unsigned)af);
    if (bits64(expected) != bits64(actual) || ef != af) {
        printf("loop %d/%d: %llx/%x != %llx/%x\n", n, changing,
            (unsigned long long)bits64(expected), ef, (unsigned long long)bits64(actual), af);
        return 1;
    }
    return 0;
}
int main(void) {
    static const uint64_t special[] = {
        0, 0x8000000000000000ULL, 1, 0x8000000000000001ULL,
        0x000fffffffffffffULL, 0x800fffffffffffffULL,
        0x0010000000000000ULL, 0x8010000000000000ULL,
        0x3ff0000000000000ULL, 0xbff0000000000000ULL,
        0x7fefffffffffffffULL, 0xffefffffffffffffULL,
        0x7ff0000000000000ULL, 0xfff0000000000000ULL,
        0x7ff8000000000055ULL, 0xfff8000000000055ULL,
        0x7ff0000000000055ULL, 0xfff0000000000055ULL
    };
    static const uint32_t fspecial[] = {
        0, 0x80000000U, 1, 0x80000001U, 0x007fffffU, 0x807fffffU,
        0x00800000U, 0x80800000U, 0x3f800000U, 0xbf800000U,
        0x7f7fffffU, 0xff7fffffU, 0x7f800000U, 0xff800000U,
        0x7fc00055U, 0xffc00055U, 0x7f800055U, 0xff800055U
    };
    static const int modes[] = {FE_TONEAREST, FE_DOWNWARD, FE_UPWARD, FE_TOWARDZERO};
    static const int trips[] = {0, 1, 2, 3, 7, 16, 32, 64};
    static const double seeds[] = {0.0, -0.0, 0.1, -0.1, 0.001, 1e-200, -1e-200, 1.0, -1.0, 1e200};
    fenv_t saved;
    if (fegetenv(&saved)) return 2;
#if defined(__x86_64__)
    unsigned csr = _mm_getcsr();
#endif
    for (unsigned m = 0; m < 4; ++m) {
        if (fesetround(modes[m])) return 2;
        for (unsigned flush = 0; flush < 4; ++flush) {
#if defined(__x86_64__)
            unsigned control = _mm_getcsr() & ~0x8040U;
            _mm_setcsr(control | ((flush & 1) ? 0x40U : 0) | ((flush & 2) ? 0x8000U : 0));
#endif
            for (unsigned j = 0; j < sizeof special / sizeof special[0]; ++j)
                if (check_double(special[j])) return 1;
            for (unsigned j = 0; j < sizeof fspecial / sizeof fspecial[0]; ++j)
                if (check_float(fspecial[j])) return 1;
            uint64_t state = 0x123456789abcdefULL;
            for (unsigned j = 0; j < 1024; ++j) {
                state ^= state << 13; state ^= state >> 7; state ^= state << 17;
                if (check_double(state) || check_float((uint32_t)state)) return 1;
            }
            for (unsigned j = 0; j < sizeof seeds / sizeof seeds[0]; ++j)
                for (unsigned k = 0; k < sizeof trips / sizeof trips[0]; ++k)
                    if (check_loop(seeds[j], trips[k], 0)) return 1;
        }
    }
    for (unsigned j = 0; j < sizeof seeds / sizeof seeds[0]; ++j)
        for (unsigned k = 0; k < sizeof trips / sizeof trips[0]; ++k)
            if (check_loop(seeds[j], trips[k], 1)) return 1;
    if (fesetenv(&saved)) return 2;
#if defined(__x86_64__)
    _mm_setcsr(csr);
#endif
    printf("fp recurrence: PASS %016llx\n", (unsigned long long)digest);
    return 0;
}
