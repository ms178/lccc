/* Two nested functions that jump to the same label of their parent.
 *
 * The label is exported once, under a name both jumps use.  Each nested
 * function used to queue that name at the label, so the assembler saw it
 * defined twice: "symbol `.LNLG.parent.out' is already defined" -- the program
 * did not build at any optimization level.
 */
#include <stdio.h>

__attribute__((noinline)) static int parent(int x)
{
    __label__ out;
    void leave(void)
    {
        goto out;
    }
    void sometimes(int y)
    {
        if (y > 1)
            goto out;
    }
    if (x == 1)
        leave();
    sometimes(x);
    return 10 + x;
out:
    return 20 + x;
}

int main(void)
{
    int got[4];
    for (int i = 0; i < 4; i++)
        got[i] = parent(i);
    printf("%d %d %d %d\n", got[0], got[1], got[2], got[3]);
    return !(got[0] == 10 && got[1] == 21 && got[2] == 22 && got[3] == 23);
}
