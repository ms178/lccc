/* Standalone ABI fixture, not kernel code: assembly is an additional caller
 * whose callback argument must not be specialized to the sole C caller's. */
static int plus_one(int x) { return x + 1; }
static int twice(int x) { return x * 2; }
static __attribute__((noinline, used)) int invoke(int (*callback)(int), int x)
{
    return callback(x);
}
extern int asm_call(int (*callback)(int), int x);
__asm__(".text\n.globl asm_call\nasm_call:\n"
#if defined(__x86_64__) || defined(__i386__)
        "jmp invoke\n"
#elif defined(__aarch64__)
        "b invoke\n"
#elif defined(__riscv)
        "j invoke\n"
#else
#error unsupported fixture architecture
#endif
        ".text\n");
static __attribute__((noinline)) int alias_target(int x) { return x + 3; }
extern int public_alias(int) __attribute__((alias("alias_target")));
int main(void)
{
    return invoke(plus_one, 11) != 12 || asm_call(twice, 13) != 26 ||
           alias_target(11) != 14 || public_alias(13) != 16;
}
