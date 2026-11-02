/* Static &&label differences must use stable named aliases: CFG cleanup may
 * renumber .LBB labels after global initializer lowering. */
#include <stdlib.h>

static int count;

__attribute__((noinline))
static void dispatch(int which)
{
    static int offsets[] = {&&one - &&base, &&two - &&base};
    goto *(&&base + offsets[which]);
one:
    count += 2;
two:
    count += 1;
base:
    return;
}

int main(void)
{
    dispatch(0);
    if (count != 3)
        abort();
    dispatch(1);
    if (count != 4)
        abort();
    return 0;
}
