/* By-value ext_vector_type calling-convention probe, built once as the
 * provider (-DPROVIDER) and once as the caller, each by lccc or by the
 * Clang oracle; see check_ext_vector_abi_grid.sh.
 *
 * -DELEM=<lane type> -DLANES=<n>; -DNO_RET drops the vector-returning
 * functions (shapes whose return lccc rejects).
 *
 * Every lane value is a small integer, so floating lanes compare exactly. */
typedef ELEM T __attribute__((ext_vector_type(LANES)));

long long hash2(int k, T a, T b, int j);
long long hashva(int n, ...);
#ifndef NO_RET
T ident(T a);
T kfour(int k, T a, T b, T c, T d, int j);
#endif

#ifdef PROVIDER
#include <stdarg.h>

static long long hash1(long long h, T v) {
    for (int i = 0; i < LANES; i++)
        h = h * 31 + (long long)v[i];
    return h;
}

long long hash2(int k, T a, T b, int j) { return hash1(hash1(k, a), b) * 7 + j; }

long long hashva(int n, ...) {
    va_list ap;
    long long h = n;
    va_start(ap, n);
    for (int i = 0; i < n; i++) {
        T v = va_arg(ap, T);
        h = hash1(h, v) * 3 + va_arg(ap, int);
    }
    va_end(ap);
    return h;
}

#ifndef NO_RET
T ident(T a) { return a; }

/* The fourth vector lands past every register piece (i386: %xmm0-2). */
T kfour(int k, T a, T b, T c, T d, int j) {
    if (k != 11 || j != 13)
        return a;
    T r = d;
    r[0] = (ELEM)(a[0] + b[0] + c[0] != 0);
    return r;
}
#endif

#else /* caller */
#include <stdio.h>

static T make(int seed) {
    T v;
    for (int i = 0; i < LANES; i++)
        v[i] = (ELEM)(((i * 7 + seed) % 5) + seed % 3);
    return v;
}

static long long hash1(long long h, T v) {
    for (int i = 0; i < LANES; i++)
        h = h * 31 + (long long)v[i];
    return h;
}

static int same(T x, T y) {
    for (int i = 0; i < LANES; i++)
        if (x[i] != y[i])
            return 0;
    return 1;
}

int main(void) {
    T a = make(1), b = make(2), c = make(3), d = make(4);
    int fails = 0;
    if (hash2(5, a, b, 9) != hash1(hash1(5, a), b) * 7 + 9) {
        puts("FAIL hash2");
        fails++;
    }
    long long want = 2;
    want = hash1(want, c) * 3 + 17;
    want = hash1(want, d) * 3 + 19;
    if (hashva(2, c, 17, d, 19) != want) {
        puts("FAIL hashva");
        fails++;
    }
#ifndef NO_RET
    if (!same(ident(a), a)) {
        puts("FAIL ident");
        fails++;
    }
    T r = d;
    r[0] = (ELEM)(a[0] + b[0] + c[0] != 0);
    if (!same(kfour(11, a, b, c, d, 13), r)) {
        puts("FAIL kfour");
        fails++;
    }
#endif
    return fails != 0;
}
#endif
