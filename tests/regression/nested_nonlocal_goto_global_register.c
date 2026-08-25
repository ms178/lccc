/* A global register assignment must write the fixed physical register, and a
 * GNU nested non-local goto restores rbp/rsp only. Restoring an entry snapshot
 * of callee-saved GPRs would erase the assignment before the target observes it. */
#include <stdlib.h>

#if defined(__x86_64__)
register void *global_ptr asm("rbx");
#else
static void *global_ptr;
#endif

int main(void)
{
    __label__ target;

    void transfer(void *value)
    {
        global_ptr = value;
        goto target;
    }

    transfer(&&target);
    return 1;

target:
    if (global_ptr != &&target)
        abort();
    return 0;
}
