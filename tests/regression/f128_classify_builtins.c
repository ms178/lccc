/* IEEE binary128 (`_Float128`) classification builtins.
 *
 * Original defect: `_Float128` and `long double` are both 16 bytes, but on
 * x86-64 they are different formats -- binary128 in an SSE register versus
 * the 80-bit x87 extended format in a memory slot -- and _Float128 reaches
 * the IR in a U128 carrier, not as an F128 float. The builtins were lowered
 * either through the libc `__*l` long-double classifiers or through the
 * f64 mask path, so `__builtin_isnan(nan)` returned 0 and
 * `__builtin_isfinite(inf)` returned 1, while `__builtin_isinf(inf)`
 * returned 0. They are now computed from the raw bit pattern, exactly.
 *
 * The expected values below are the gcc-16.2 ones, except signbit where the
 * standard only requires "non-zero": lccc returns the sign bit itself (1).
 */
#include <stdio.h>

static int cls(_Float128 x) {
    return __builtin_fpclassify(0, 1, 2, 3, 4, x);
}

int main(void) {
    volatile _Float128 zero = 0.0Q;
    _Float128 nan = zero / zero;
    _Float128 inf = 1.0Q / zero;
    _Float128 ninf = -1.0Q / zero;
    _Float128 one = 1.0Q;
    _Float128 neg = -1.0Q;
    int fails = 0;

    if (!__builtin_isnan(nan)) { printf("FAIL isnan(nan)\n"); fails++; }
    if (__builtin_isnan(one)) { printf("FAIL isnan(one)\n"); fails++; }
    if (__builtin_isinf(nan)) { printf("FAIL isinf(nan)\n"); fails++; }
    if (!__builtin_isinf(inf) || !__builtin_isinf(ninf)) { printf("FAIL isinf(inf)\n"); fails++; }
    if (__builtin_isfinite(inf) || __builtin_isfinite(nan)) { printf("FAIL isfinite\n"); fails++; }
    if (!__builtin_isfinite(one) || !__builtin_isfinite(zero)) { printf("FAIL isfinite(finite)\n"); fails++; }
    if (!__builtin_isnormal(one) || __builtin_isnormal(nan) || __builtin_isnormal(inf)
        || __builtin_isnormal(zero)) {
        printf("FAIL isnormal\n");
        fails++;
    }
    if (__builtin_signbit(one) || !__builtin_signbit(neg) || !__builtin_signbit(ninf)) {
        printf("FAIL signbit\n");
        fails++;
    }
    if (__builtin_isinf_sign(one) != 0 || __builtin_isinf_sign(inf) != 1
        || __builtin_isinf_sign(ninf) != -1 || __builtin_isinf_sign(nan) != 0) {
        printf("FAIL isinf_sign %d %d %d %d\n",
               __builtin_isinf_sign(one), __builtin_isinf_sign(inf),
               __builtin_isinf_sign(ninf), __builtin_isinf_sign(nan));
        fails++;
    }
    if (cls(nan) != 0 || cls(inf) != 1 || cls(one) != 2 || cls(zero) != 4
        || cls(neg) != 2) {
        printf("FAIL fpclassify %d %d %d %d %d\n",
               cls(nan), cls(inf), cls(one), cls(zero), cls(neg));
        fails++;
    }

    /* The x87 long double classifier must keep working: it is a different
     * format and still routed through libc. */
    volatile long double lz = 0.0L;
    long double ln = lz / lz, li = 1.0L / lz;
    if (!__builtin_isnan(ln) || !__builtin_isinf(li) || __builtin_isfinite(li)) {
        printf("FAIL long double classify\n");
        fails++;
    }
    if (fails == 0) {
        printf("PASS f128_classify_builtins\n");
    }
    return fails != 0;
}
