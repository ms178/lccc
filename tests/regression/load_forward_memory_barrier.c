/* A conditional reload must not reuse a value from before an intervening
 * memory-writing asm, whether the asm precedes or follows the branch. */
#include <stdio.h>

#if defined(__x86_64__) || defined(__i386__)
#define WRITE_SEVENTEEN(p) __asm__ volatile("movl $17, (%0)" : : "r"(p) : "memory")
#else
/* Keep the same behavioral contract on targets without the x86 spelling. */
#define WRITE_SEVENTEEN(p) (*(p) = 17u)
#endif

__attribute__((noinline))
static unsigned predecessor_write(unsigned *p) {
    unsigned old = *p;
    WRITE_SEVENTEEN(p);
    if (old != 0)
        return *p;
    return old;
}

__attribute__((noinline))
static unsigned successor_write(unsigned *p) {
    unsigned old = *p;
    if (old != 0) {
        WRITE_SEVENTEEN(p);
        return *p;
    }
    return old;
}

int main(void) {
    unsigned value = 1;
    if (predecessor_write(&value) != 17 || value != 17)
        return 1;
    value = 1;
    if (successor_write(&value) != 17 || value != 17)
        return 2;
    value = 0;
    if (predecessor_write(&value) != 0 || value != 17)
        return 3;
    value = 0;
    if (successor_write(&value) != 0 || value != 0)
        return 4;
    puts("load_forward_memory_barrier: ok");
    return 0;
}
