/* General complete unrolling must not clone a pointer-controlled outer loop
 * whose nested loop carries mutable object state as though every iteration
 * still saw iteration zero's pointer.  Until that state has an explicit
 * memory-SSA representation, this shape remains looped. */
#include <stdlib.h>

int main(void)
{
    int left = 10;
    int right = 20;
    int *cursor = &left;
    int visits = 0;

    for (int outer = 0; outer < 10; ++outer) {
        cursor = cursor == &left ? &right : &left;
        while ((*cursor)--) {
            ++visits;
            if (*cursor < 3)
                break;
            cursor = &right;
        }
        ++visits;
        cursor = &right;
    }

    if (*cursor != -5 || right != -5 || visits != 43)
        abort();
    return 0;
}
