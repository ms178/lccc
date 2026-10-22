/* Stack-passed 32-byte vectors must arrive whole.
 *
 * The ninth and tenth __m256-class arguments travel on the stack; the callee
 * copies them into its frame in 8-byte chunks and reads each slot back with
 * one 32-byte `vmovups ..., %ymm`.  A peephole classification that modeled
 * that YMM load as a 16-byte access let write-region liveness prove the
 * upper-half chunk stores dead and delete them, so the upper lanes of the
 * stack arguments were uninitialised (the lower lanes were exact, which is
 * what makes the shape worth pinning: lane 4..7 differ, lane 0..3 agree).
 * Requires: -mavx2. */
#include <stdio.h>

typedef float v8f __attribute__((vector_size(32)));
typedef double v4d __attribute__((vector_size(32)));

__attribute__((noinline))
v8f spill(v8f a, v8f b, v8f c, v8f d, v8f e, v8f f, v8f g, v8f h, v8f i, v8f j,
          int k, v4d z)
{
    v8f r = a + b + c + d + e + f + g + h + i + j;
    r[7] += (float)k;
    r[6] += (float)z[3];
    return r;
}

int main(void)
{
    v8f a = {1, 2, 3, 4, 5, 6, 7, 8};
    v8f b = {10, 20, 30, 40, 50, 60, 70, 80};
    v4d z = {0.5, 1.5, 2.5, 3.5};
    v8f r = spill(a, b, a, b, a, b, a, b, a, b, 42, z);
    printf("%.3f %.3f %.3f %.3f %.3f %.3f %.3f %.3f\n",
           (double)r[0], (double)r[1], (double)r[2], (double)r[3],
           (double)r[4], (double)r[5], (double)r[6], (double)r[7]);
    return 0;
}
