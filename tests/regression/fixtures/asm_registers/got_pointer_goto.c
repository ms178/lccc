/* i686, position-independent code: `asm goto` that takes %ebx.

   The GOT pointer has to be back in %ebx on every way out of the statement:
   behind it, and at each label it can jump to. A statement without outputs
   used to jump straight to the label; now each edge has a block of its own
   that reloads the pointer first. Every function reads a global and calls
   through the PLT on both ways out. */
#include <stdio.h>
#include <string.h>
int counter = 5;
char text[16] = "abcdefgh";
#define NOINLINE __attribute__((noinline))

/* no outputs: the edges had no blocks of their own */
NOINLINE int branch_in(int x) {
    __asm__ goto("testl %%ebx, %%ebx\n\tjz %l[zero]" : : "b"(x) : "cc" : zero);
    return (int)strlen(text) + counter;
zero:
    return -(int)strlen(text) - counter;
}

/* clobber only */
NOINLINE int branch_clobber(int x) {
    __asm__ goto("xorl %%ebx, %%ebx\n\tcmpl $7, %0\n\tje %l[seven]" : : "r"(x) : "ebx", "cc" : seven);
    return (int)strlen(text) + counter + x;
seven:
    return 1000 + (int)strlen(text) + counter;
}

/* an output in ebx on both ways out */
NOINLINE int branch_out(int x) {
    int r;
    __asm__ goto("leal 1(%1), %%ebx\n\tcmpl $3, %1\n\tjg %l[big]" : "=b"(r) : "r"(x) : "cc" : big);
    return r + (int)strlen(text) + counter;
big:
    return 10 * r + (int)strlen(text) + counter;
}

/* in a loop, the label behind the loop */
NOINLINE int loop_exit(int n) {
    int acc = 0;
    for (int i = 0; i < n; i++) {
        int v = i + counter;
        __asm__ goto("addl $2, %%ebx\n\tcmpl $12, %%ebx\n\tjge %l[out]" : "+b"(v) : : "cc" : out);
        acc += v + (unsigned char)text[i & 7];
    }
    return acc + (int)strlen(text);
out:
    return -acc - (int)strlen(text) - counter;
}

int main(void) {
    printf("branch_in %d %d\n", branch_in(0), branch_in(4));
    printf("branch_clobber %d %d\n", branch_clobber(7), branch_clobber(9));
    printf("branch_out %d %d\n", branch_out(1), branch_out(8));
    printf("loop_exit %d %d\n", loop_exit(3), loop_exit(40));
    return 0;
}
