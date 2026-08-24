/* A nested function captures a VLA as a runtime descriptor: dynamic base plus
 * byte size. Both indexing and sizeof(VLA) must refer to the parent's live
 * allocation rather than an undefined global or copied fixed-size object. */
#include <stdlib.h>

__attribute__((noinline))
static long exercise(int count)
{
    int values[count];
    values[2] = 17;

    long inspect(int index)
    {
        return (long)sizeof(values) + values[index];
    }

    return inspect(2);
}

int main(void)
{
    if (exercise(7) != (long)(7 * sizeof(int) + 17))
        abort();
    return 0;
}
