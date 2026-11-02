/* i686 exercises the accumulator backend that originally needed the spill workaround. */
#define main post_inc_test_main
#include "post_inc_ssa.c"
#undef main

__attribute__((noreturn)) void _start(void)
{
    int status = post_inc_test_main();
    __asm__ volatile ("int $0x80" : : "a"(1), "b"(status) : "memory");
    __builtin_unreachable();
}
