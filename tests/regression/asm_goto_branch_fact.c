/* The C branch and asm goto can enter the same block. Only the C branch
 * proves x is nonzero there. Exercise both values on the asm edge. */
#if defined(__i386__) || defined(__x86_64__) || defined(__aarch64__) || defined(__riscv)
#if defined(__i386__) || defined(__x86_64__)
#define JUMP "jmp "
#elif defined(__aarch64__)
#define JUMP "b "
#else
#define JUMP "j "
#endif

__attribute__((noinline)) int check_asm_edge(int x)
{
    asm goto(JUMP "%l[fast]" : : : : fast);
    if (x)
        goto fast;
    return 40;
fast:
    return x ? 41 : 42;
}

int main(void)
{
    volatile int zero = 0;
    return check_asm_edge(zero) != 42 || check_asm_edge(zero + 1) != 41;
}
#else
int main(void) { return 0; }
#endif
