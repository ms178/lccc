/* Strong definitions replacing caller.c's weak leaves at link time. */
#define LEAF(name)                                                         \
    long name(long x) {                                                    \
        long r;                                                            \
        __asm__ volatile("movq $-1, %%r8\n\tmovq $-1, %%r9\n\t"            \
                         "movq $-1, %%r10\n\tmovq $-1, %%r11\n\t"          \
                         "leaq 1(%1,%1,2), %0"                             \
                         : "=r"(r) : "r"(x) : "r8", "r9", "r10", "r11");   \
        return r;                                                          \
    }
LEAF(leaf_pragma)
LEAF(leaf_late)
LEAF(leaf_asm)
