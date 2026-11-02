/* `fold_lea_into_memory_op` once composed a LEA producer into a consumer that
 * names the produced register twice (`leaq -3(%rdi),%T; leaq (%T,%T,4),%T` ->
 * `leaq -3(%rdi,%T,4),%T`), leaving a stale index.  Both operation widths and
 * several scales are exercised; every value is used after the multiply. */
#include <stdio.h>

#define N 64

__attribute__((noinline)) static void m32(int *restrict o, const int *restrict a, int n) {
    for (int i = 0; i < n; i++) o[i] = (i - 3) * 5 + a[i];
}
__attribute__((noinline)) static void m32b(unsigned *restrict o, const unsigned *restrict a, unsigned n) {
    for (unsigned i = 0; i < n; i++) o[i] = (i + 7u) * 9u + a[i];
}
__attribute__((noinline)) static void m64(long *restrict o, const long *restrict a, long n) {
    for (long i = 0; i < n; i++) o[i] = (i - 3) * 5 + 7 + a[i];
}
__attribute__((noinline)) static long m64s(long i) { return (i + 11) * 3; }
__attribute__((noinline)) static int m32s(int i) { return (i - 13) * 9; }

/* Reference: the products built by repeated addition (no LEA idiom). */
__attribute__((noinline)) static long times(long x, int k) {
    long r = 0;
    for (int j = 0; j < k; j++) r += x;
    return r;
}

int main(void) {
    int a[N], o[N];
    unsigned ua[N], uo[N];
    long la[N], lo[N];
    for (int i = 0; i < N; i++) { a[i] = i * 31 - 500; ua[i] = (unsigned)i * 2654435761u; la[i] = (long)i * -977; }
    m32(o, a, N);
    m32b(uo, ua, N);
    m64(lo, la, N);
    unsigned long sum = 0;
    int bad = 0;
    for (int i = 0; i < N; i++) {
        bad |= o[i] != (int)times(i - 3, 5) + a[i];
        bad |= uo[i] != (unsigned)times(i + 7, 9) + ua[i];
        bad |= lo[i] != times(i - 3, 5) + 7 + la[i];
        sum = sum * 1000003u + (unsigned)o[i] + uo[i] + (unsigned long)lo[i];
    }
    for (long i = -40; i <= 40; i += 7) {
        bad |= m64s(i) != times(i + 11, 3);
        bad |= m32s((int)i) != (int)times(i - 13, 9);
        sum = sum * 31u + (unsigned long)m64s(i) + (unsigned)m32s((int)i);
    }
    printf("sum=%lu\n", sum);
    if (bad) { puts("FAIL"); return 1; }
    puts("ok");
    return 0;
}
