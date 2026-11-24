/* A reduction whose loop is nested in another loop that carries the sum.
 *
 * The inner loop's accumulator starts at the outer loop's running value and
 * leaves through the inner loop's header, so the value that is live after
 * the inner loop is the header's own phi, read by the OUTER loop's phi on
 * its back edge:
 *
 *     for (i = 0; i < n; i += 64)
 *         for (j = 0; j < 64; j++)
 *             sum += a[i + j];
 *
 * The byte-sum vectorizer turns the inner accumulator into a vector of
 * partial sums and has to hand the reduced value, plus the value the loop
 * started from, to every reader outside the loop. It left the outer phi
 * reading the accumulator it had just turned into a vector, because the
 * outer phi also feeds the reduced value (it is the start value): the rule
 * that keeps a rewrite from feeding a value to itself took the recurrence
 * of the enclosing loop for such a cycle. f() returned the vector register
 * where its caller reads %rax: 0, or whatever the register held.
 *
 * Every function is a shape of that: other element and sum types, other
 * trip counts (with and without a remainder), a use between the loops, a
 * second accumulator, three levels, an inner loop under a condition, and
 * the counting form. GCC computes them all; the runner compares.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define NOINLINE __attribute__((noinline))

NOINLINE unsigned long bytes_to_u64(const unsigned char *a, int n)
{
    unsigned long sum = 0;
    for (int i = 0; i < n; i += 64)
        for (int j = 0; j < 64; j++)
            sum += a[i + j];
    return sum;
}

NOINLINE unsigned bytes_to_u32(const unsigned char *a, int n)
{
    unsigned sum = 7;
    for (int i = 0; i < n; i += 32)
        for (int j = 0; j < 32; j++)
            sum += a[i + j];
    return sum;
}

NOINLINE long schar_to_long(const signed char *a, int n)
{
    long sum = -3;
    for (int i = 0; i < n; i += 64)
        for (int j = 0; j < 64; j++)
            sum += a[i + j];
    return sum;
}

/* A trip count with a remainder (40 = 2 * 16 + 8). */
NOINLINE unsigned long bytes_remainder(const unsigned char *a, int n)
{
    unsigned long sum = 0;
    for (int i = 0; i + 40 <= n; i += 40)
        for (int j = 0; j < 40; j++)
            sum += a[i + j];
    return sum;
}

/* The inner trip count is the program's. */
NOINLINE unsigned long bytes_variable_inner(const unsigned char *a, int rows, int width)
{
    unsigned long sum = 0;
    for (int i = 0; i < rows; i++)
        for (int j = 0; j < width; j++)
            sum += a[i * width + j];
    return sum;
}

/* The sum is read and changed between the two loops. */
NOINLINE unsigned long bytes_use_between(const unsigned char *a, int n)
{
    unsigned long sum = 1;
    for (int i = 0; i < n; i += 64) {
        for (int j = 0; j < 64; j++)
            sum += a[i + j];
        sum = sum * 3 + (unsigned long)i;
    }
    return sum;
}

/* The sum is changed in front of the inner loop too. */
NOINLINE unsigned long bytes_use_around(const unsigned char *a, int n)
{
    unsigned long sum = 5;
    for (int i = 0; i < n; i += 64) {
        sum ^= (unsigned long)i << 3;
        for (int j = 0; j < 64; j++)
            sum += a[i + j];
        sum += 1;
    }
    return sum;
}

/* Two sums in one inner loop. */
NOINLINE unsigned long bytes_two_sums(const unsigned char *a, const unsigned char *b, int n)
{
    unsigned long x = 0, y = 100;
    for (int i = 0; i < n; i += 64)
        for (int j = 0; j < 64; j++) {
            x += a[i + j];
            y += b[i + j];
        }
    return x * 1000003u + y;
}

/* Three levels: the sum is carried by both enclosing loops. */
NOINLINE unsigned long bytes_three_levels(const unsigned char *a, int planes, int rows)
{
    unsigned long sum = 0;
    for (int p = 0; p < planes; p++)
        for (int r = 0; r < rows; r++)
            for (int j = 0; j < 48; j++)
                sum += a[(p * rows + r) * 48 + j];
    return sum;
}

/* The inner loop runs under a condition: the sum merges with itself. */
NOINLINE unsigned long bytes_conditional_inner(const unsigned char *a, int n)
{
    unsigned long sum = 0;
    for (int i = 0; i < n; i += 64) {
        if (a[i] & 1)
            for (int j = 0; j < 64; j++)
                sum += a[i + j];
        else
            sum += 2;
    }
    return sum;
}

/* The counting form: how many bytes equal a value. */
NOINLINE unsigned long bytes_count_equal(const unsigned char *a, int n, unsigned char c)
{
    unsigned long count = 0;
    for (int i = 0; i < n; i += 64)
        for (int j = 0; j < 64; j++)
            count += a[i + j] == c;
    return count;
}

NOINLINE int bytes_count_above(const unsigned char *a, int n)
{
    int count = 11;
    for (int i = 0; i < n; i += 128)
        for (int j = 0; j < 128; j++)
            if (a[i + j] > 127)
                count++;
    return count;
}

/* The sum of each block is stored, and the total is carried as well. */
NOINLINE unsigned long bytes_store_each(const unsigned char *a, int n, unsigned long *out)
{
    unsigned long total = 0;
    for (int i = 0; i < n; i += 64) {
        unsigned long block = 0;
        for (int j = 0; j < 64; j++)
            block += a[i + j];
        out[i / 64] = block;
        total += block;
    }
    return total;
}

/* The same reductions over wider elements. */
NOINLINE long ints_to_long(const int *a, int n)
{
    long sum = 0;
    for (int i = 0; i < n; i += 16)
        for (int j = 0; j < 16; j++)
            sum += a[i + j];
    return sum;
}

NOINLINE int ints_to_int(const int *a, int n)
{
    int sum = 9;
    for (int i = 0; i < n; i += 16)
        for (int j = 0; j < 16; j++)
            sum += a[i + j];
    return sum;
}

NOINLINE unsigned ushorts_to_u32(const unsigned short *a, int n)
{
    unsigned sum = 0;
    for (int i = 0; i < n; i += 32)
        for (int j = 0; j < 32; j++)
            sum += a[i + j];
    return sum;
}

NOINLINE int ints_xor(const int *a, int n)
{
    int x = 0x55;
    for (int i = 0; i < n; i += 16)
        for (int j = 0; j < 16; j++)
            x ^= a[i + j];
    return x;
}

NOINLINE int ints_max(const int *a, int n)
{
    int m = -2147483647 - 1;
    for (int i = 0; i < n; i += 16)
        for (int j = 0; j < 16; j++)
            if (a[i + j] > m)
                m = a[i + j];
    return m;
}

NOINLINE double doubles_sum(const double *a, int n)
{
    double sum = 0.5;
    for (int i = 0; i < n; i += 8)
        for (int j = 0; j < 8; j++)
            sum += a[i + j];
    return sum;
}

/* The inner loop inlined into its caller: the sum of the caller's loop. */
static inline unsigned long block_sum(const unsigned char *p)
{
    unsigned long s = 0;
    for (int j = 0; j < 64; j++)
        s += p[j];
    return s;
}

NOINLINE unsigned long bytes_inlined_helper(const unsigned char *a, int n)
{
    unsigned long sum = 0;
    for (int i = 0; i < n; i += 64)
        sum += block_sum(a + i);
    return sum;
}

/* The block behind the loop is the head of the next loop. */
NOINLINE unsigned long bytes_then_loop(const unsigned char *a, int k)
{
    unsigned long sum = 3, x = 0;
    for (int j = 0; j < 64; j++)
        sum += a[j];
    do {
        x += sum;
        sum += 1;
    } while (--k > 0);
    return x;
}

/* The index is read behind the loop, with the sum. */
NOINLINE unsigned long bytes_index_after(const unsigned char *a, unsigned n)
{
    unsigned long sum = 0;
    unsigned j;
    for (j = 0; j < n; j++)
        sum += a[j];
    return sum * 31 + j;
}

NOINLINE long bytes_index_after_signed(const unsigned char *a, int n)
{
    long sum = 0;
    int j;
    for (j = 0; j < n; j++)
        sum += a[j];
    return sum * 31 + j;
}

NOINLINE unsigned long bytes_index_after_const(const unsigned char *a)
{
    unsigned long sum = 0;
    unsigned j;
    for (j = 0; j < 100; j++)
        sum += a[j];
    return sum * 31 + j;
}

/* The loop under a test of its trip count: the sum merges with its start. */
NOINLINE unsigned bytes_guarded(const unsigned char *a, int n, unsigned start)
{
    unsigned sum = start;
    if (n > 0) {
        for (int j = 0; j < n; j++)
            sum += a[j];
        sum ^= 0x55;
    }
    return sum;
}

/* A sum narrower than the register, merged with another path. */
NOINLINE unsigned short bytes_narrow_merge(const unsigned char *a, int n, int c)
{
    unsigned short sum = 9;
    if (c)
        for (int j = 0; j < n; j++)
            sum += a[j];
    else
        sum = 77;
    return sum;
}

/* The sum of one loop is the start of the next. */
NOINLINE unsigned long bytes_two_loops(const unsigned char *a, const unsigned char *b, int n)
{
    unsigned long sum = 0;
    for (int j = 0; j < n; j++)
        sum += a[j];
    for (int j = 0; j < n; j++)
        sum += b[j];
    return sum;
}

int main(void)
{
    enum { N = 1 << 13 };
    unsigned char *a = malloc(N), *b = malloc(N);
    int *w = malloc(N * sizeof *w);
    unsigned short *h = malloc(N * sizeof *h);
    double *d = malloc(N * sizeof *d);
    unsigned long *out = malloc((N / 64) * sizeof *out);
    if (!a || !b || !w || !h || !d || !out)
        return 2;
    unsigned state = 12345;
    for (int i = 0; i < N; i++) {
        state = state * 1103515245u + 12345u;
        a[i] = (unsigned char)(state >> 16);
        b[i] = (unsigned char)(state >> 9);
        w[i] = (int)((state >> 8) % 200001u) - 100000;
        h[i] = (unsigned short)(state >> 5);
        d[i] = (double)((int)(state >> 12) % 1000) * 0.25;
    }

    /* Each function at several sizes, the empty one among them. */
    static const int sizes[] = { 0, 64, 128, 640, 1 << 12, N };
    for (unsigned k = 0; k < sizeof sizes / sizeof sizes[0]; k++) {
        int n = sizes[k];
        printf("n=%d\n", n);
        printf(" bytes_to_u64 %lu\n", bytes_to_u64(a, n));
        printf(" bytes_to_u32 %u\n", bytes_to_u32(a, n));
        printf(" schar_to_long %ld\n", schar_to_long((const signed char *)a, n));
        printf(" bytes_remainder %lu\n", bytes_remainder(a, n));
        printf(" bytes_variable_inner %lu %lu %lu\n", bytes_variable_inner(a, n / 64, 64),
               bytes_variable_inner(a, n / 100, 100), bytes_variable_inner(a, n / 7, 7));
        printf(" bytes_use_between %lu\n", bytes_use_between(a, n));
        printf(" bytes_use_around %lu\n", bytes_use_around(a, n));
        printf(" bytes_two_sums %lu\n", bytes_two_sums(a, b, n));
        printf(" bytes_three_levels %lu %lu\n", bytes_three_levels(a, n / 480, 10),
               bytes_three_levels(a, 1, n / 48));
        printf(" bytes_conditional_inner %lu\n", bytes_conditional_inner(a, n));
        printf(" bytes_count_equal %lu %lu\n", bytes_count_equal(a, n, 0),
               bytes_count_equal(a, n, 0x5a));
        printf(" bytes_count_above %d\n", bytes_count_above(a, n));
        memset(out, 0, (N / 64) * sizeof *out);
        unsigned long total = bytes_store_each(a, n, out);
        unsigned long folded = 0;
        for (int i = 0; i < N / 64; i++)
            folded = folded * 31 + out[i];
        printf(" bytes_store_each %lu %lu\n", total, folded);
        printf(" ints_to_long %ld\n", ints_to_long(w, n));
        printf(" ints_to_int %d\n", ints_to_int(w, n));
        printf(" ushorts_to_u32 %u\n", ushorts_to_u32(h, n));
        printf(" ints_xor %d\n", ints_xor(w, n));
        printf(" ints_max %d\n", ints_max(w, n));
        printf(" doubles_sum %.2f\n", doubles_sum(d, n));
        printf(" bytes_inlined_helper %lu\n", bytes_inlined_helper(a, n));
        printf(" bytes_then_loop %lu %lu\n", bytes_then_loop(a + (n & 63), 1),
               bytes_then_loop(a + (n & 63), 5));
        printf(" bytes_index_after %lu %lu\n", bytes_index_after(a, (unsigned)n),
               bytes_index_after(a, (unsigned)n / 3 + 5));
        printf(" bytes_index_after_signed %ld %ld %ld\n", bytes_index_after_signed(a, n),
               bytes_index_after_signed(a, n / 3 + 5), bytes_index_after_signed(a, -n));
        printf(" bytes_index_after_const %lu\n", bytes_index_after_const(a + (n & 255)));
        printf(" bytes_guarded %u %u %u\n", bytes_guarded(a, n, 1u), bytes_guarded(a, n / 9, 2u),
               bytes_guarded(a, -n, 3u));
        printf(" bytes_narrow_merge %u %u %u\n", (unsigned)bytes_narrow_merge(a, n, 1),
               (unsigned)bytes_narrow_merge(a, n / 5 + 3, 1), (unsigned)bytes_narrow_merge(a, n, 0));
        printf(" bytes_two_loops %lu\n", bytes_two_loops(a, b, n));
    }
    free(a); free(b); free(w); free(h); free(d); free(out);
    return 0;
}
