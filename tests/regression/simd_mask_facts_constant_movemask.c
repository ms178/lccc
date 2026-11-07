/* Constant byte mask -> movemask.  The whole image is provable, so the
 * compiler may reduce the function to a single immediate.  GCC 16.2 and
 * Clang 23.1.5 emit `movl $53069, %eax; ret` at -O2. */
#include <emmintrin.h>
#include <stdio.h>
int main(void) {
    __m128i v = _mm_setr_epi8(
        -1, 0, -1, -1, 0, 0, -1, 0,
        -1, -1, -1, -1, 0, 0, -1, -1);
    printf("%d\n", _mm_movemask_epi8(v));
    return 0;
}
