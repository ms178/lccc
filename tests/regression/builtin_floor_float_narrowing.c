/* gcc.c-torture/execute/20030125-1.c regression.
 *
 * floor((double)x) is exactly floorf(x) when x is a float.  Both the plain
 * library spelling and __builtin_floor must use that contraction even when
 * this TU supplies weak test implementations of the two library names.
 * Before the fix LCCC retained the double call and reached floor(), which
 * aborts here, at every optimizing level on x86-64 and i686.
 */
void abort(void);
double floor(double);
float floorf(float);

volatile float zero;

__attribute__((noinline)) float plain_float(float x) { return floor(x); }
__attribute__((noinline)) double plain_double(float x) { return floor(x); }
__attribute__((noinline)) float builtin_float(float x) {
    return __builtin_floor(x);
}
__attribute__((noinline)) double builtin_double(float x) {
    return __builtin_floor(x);
}

int main(void) {
    float x = zero;
    if (plain_float(x) != 0.0f || plain_double(x) != 0.0)
        abort();
    if (builtin_float(x) != 0.0f || builtin_double(x) != 0.0)
        abort();
    return 0;
}

__attribute__((weak)) double floor(double x) {
    (void)x;
    abort();
    return 0.0;
}

__attribute__((weak)) float floorf(float x) { return x; }
