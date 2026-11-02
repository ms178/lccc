__attribute__((noinline)) static void smash_caller_saved(void)
{
    __asm__ volatile("movl $99, %%eax; movl $88, %%edx; movl $77, %%ecx"
                     ::: "eax", "edx", "ecx");
}

__attribute__((noinline)) static int weighted(int a, int b, int c)
{
    smash_caller_saved();
    return a + 10 * b + 100 * c;
}

__attribute__((noinline)) static int narrow(signed char a, unsigned char b,
                                             short c)
{
    smash_caller_saved();
    return a + 10 * b + 100 * c;
}

void _start(void)
{
    int ok = weighted(1, 2, 3) == 321 && narrow(-2, 250, -3) == 2198;
    __asm__ volatile("int $0x80" :: "a"(1), "b"(ok ? 0 : 1) : "memory");
    __builtin_unreachable();
}
