/* Tail-recursion-to-loop must not treat `r = f(...); <side effect>; return r;`
 * as a tail call.  tail_calls_to_loops turned the self call into a back
 * edge but left the trailing instructions in place, so they ran before the
 * callee's body: at -O2 every `counter` update (and a volatile asm after the
 * call) executed before the base case, which then saw counter == 10.  */
#include <stdio.h>

int counter, seen = -1;
unsigned trace[16], ntrace;

__attribute__((noinline)) int depth(int n)
{
    if (n == 0) {
        seen = counter;
        return 7;
    }
    int r = depth(n - 1);
    counter++;
    return r;
}

__attribute__((noinline)) int depth_asm(int n)
{
    if (n == 0) {
        seen = counter;
        return 9;
    }
    int r = depth_asm(n - 1);
    __asm__ volatile("incl %0" : "+m"(counter));
    return r;
}

__attribute__((noinline)) void order(unsigned n)
{
    if (n == 0)
        return;
    order(n - 1);
    trace[ntrace++] = n; /* post-order: 1, 2, ..., n */
}

int main(void)
{
    int bad = 0;
    int r = depth(10);
    if (seen != 0 || counter != 10 || r != 7) {
        printf("depth: seen=%d counter=%d r=%d\n", seen, counter, r);
        bad = 1;
    }
    counter = 0;
    seen = -1;
    r = depth_asm(10);
    if (seen != 0 || counter != 10 || r != 9) {
        printf("depth_asm: seen=%d counter=%d r=%d\n", seen, counter, r);
        bad = 1;
    }
    order(5);
    for (unsigned i = 0; i < 5; i++)
        if (trace[i] != i + 1) {
            printf("order: trace[%u]=%u\n", i, trace[i]);
            bad = 1;
        }
    return bad;
}
