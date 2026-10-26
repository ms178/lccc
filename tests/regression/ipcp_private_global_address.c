/* Uniform private addresses are constants; their contents are NOT constants.
 * Distinct/unknown pointers, escaping entries, TLS and interposable objects
 * must retain ordinary parameter semantics. */
#include <stdio.h>
static int a[8], b[8];
static volatile int touches;
static _Thread_local int thread_value;
int public_value;
static __attribute__((noinline)) int uniform(int *p, int n) {
    int sum = 0;
    for (int i = 0; i < n; ++i) sum += p[i];
    return sum;
}
static __attribute__((noinline)) int partial(int *p, int n) {
    int sum = 0;
    for (int i = 0; i < n; ++i) sum += p[i];
    return sum;
}
static __attribute__((noinline)) int offset_read(int *p) { return *p; }
static __attribute__((noinline)) int varying(int *p) { return *p; }
static __attribute__((noinline)) int escaped(int *p) { return *p + 17; }
static int (*volatile escape)(int *) = escaped;
static __attribute__((noinline)) int tls_read(int *p) { return *p; }
static __attribute__((noinline)) int public_read(int *p) { return *p; }
int main(void) {
    for (int i = 0; i < 8; ++i) { a[i] = i; b[i] = 20 + i; }
    int first = uniform((touches++, a), 8);
    a[0] = 100;
    int second = uniform((touches++, a), 8);
    int small = partial(a, 4), large = partial(a, 8);
    int base = offset_read(a), offset = offset_read(a + 1);
    int x = varying(a), y = varying(b);
    int z = escaped(a), w = escape(b);
    thread_value = 43;
    int t = tls_read(&thread_value);
    thread_value = 71;
    int u = tls_read(&thread_value);
    public_value = 91;
    int v = public_read(&public_value);
    public_value = 123;
    int q = public_read(&public_value);
    if (small != 106 || large != 128 || base != 100 || offset != 1 ||
        touches != 2 || first != 28 || second != 128 || x != 100 || y != 20 ||
        z != 117 || w != 37 || t != 43 || u != 71 || v != 91 || q != 123)
        return 1;
    puts("private address, mutable contents, varying pointers, escape, TLS: PASS");
    return 0;
}
