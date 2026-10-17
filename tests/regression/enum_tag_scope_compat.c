/*
 * A completed enum tag is a scope-bound mapping, not a global name cache.
 * GCC chooses `int` for an enum with a negative value and `unsigned int` for
 * a non-negative enum. Both local definitions below shadow outer tags; once
 * their compound statements end, references must resolve to the outer enum
 * definition and preserve its compatible pointer type.
 */
#include <stdio.h>

enum signed_tag { SIGNED_NEGATIVE = -1 };
enum unsigned_tag { UNSIGNED_ZERO = 0 };

static int read_int(int *p) { return *p; }
static unsigned int read_uint(unsigned int *p) { return *p; }

int main(void)
{
    {
        enum signed_tag { INNER_SIGNED_ZERO = 0 };
        enum signed_tag inner = INNER_SIGNED_ZERO;
        if (inner != 0)
            return 1;
    }
    {
        enum unsigned_tag { INNER_UNSIGNED_NEGATIVE = -1 };
        enum unsigned_tag inner = INNER_UNSIGNED_NEGATIVE;
        if (inner != -1)
            return 2;
    }

    /* These body-less references are after the shadowing scopes have ended. */
    enum signed_tag signed_value = SIGNED_NEGATIVE;
    enum unsigned_tag unsigned_value = UNSIGNED_ZERO;
    if (read_int(&signed_value) != -1)
        return 3;
    if (read_uint(&unsigned_value) != 0)
        return 4;

    puts("outer enum definitions restored");
    return 0;
}
