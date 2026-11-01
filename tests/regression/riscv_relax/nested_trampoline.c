/*
 * A GNU C nested function whose address escapes: GCC writes a trampoline on
 * the stack and marks the object `.note.GNU-stack,"x"`, so the linker must
 * give the program an executable stack (PT_GNU_STACK RWX, GNU ld's rule).
 * Linked with a non-executable stack it faults on the first indirect call.
 */
#include <stdio.h>

__attribute__((noinline)) static int apply(int (*volatile fn)(int), int v)
{
    return fn(v);
}

__attribute__((noinline)) static int run(int bias)
{
    int add(int x) { return x + bias; }
    return apply(add, 41);
}

int main(void)
{
    int r = run(1);
    printf("%d\n", r);
    return r == 42 ? 0 : 1;
}
