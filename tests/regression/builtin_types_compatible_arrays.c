/* __builtin_types_compatible_p follows C composite-type rules for arrays:
 * an incomplete bound is compatible with any completed bound, while two
 * different known bounds are incompatible. */
#include <stdlib.h>

typedef int five_ints[5];
typedef int six_ints[6];
typedef int unknown_ints[];

int main(void)
{
    /* Top-level qualification is ignored; pointee qualification is not. */
    if (!__builtin_types_compatible_p(const int, int))
        abort();
    if (!__builtin_types_compatible_p(char * const, char *))
        abort();
    if (__builtin_types_compatible_p(const char *, char *))
        abort();

    if (!__builtin_types_compatible_p(five_ints, int[5]))
        abort();
    if (!__builtin_types_compatible_p(five_ints, unknown_ints))
        abort();
    if (!__builtin_types_compatible_p(int[], int[17]))
        abort();
    if (__builtin_types_compatible_p(five_ints, six_ints))
        abort();
    if (__builtin_types_compatible_p(int[], unsigned int[]))
        abort();
    return 0;
}
