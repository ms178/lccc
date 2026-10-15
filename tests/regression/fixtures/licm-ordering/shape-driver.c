/* Single-threaded cost witness: deliberately uncontended, not a concurrency
   latency benchmark. Kernel is a separate object, with no LTO or constant globals. */
#include <stdio.h>
int ordering_g0 = 1, ordering_g1 = 2, ordering_g2 = 3, ordering_g3 = 4;
unsigned ordering_ready, ordering_done = 1;
#ifndef ORDERING_FUNCTION
#define ORDERING_FUNCTION ordering_plain_shape
#endif
extern unsigned ORDERING_FUNCTION(unsigned);
int main(void) {
    unsigned result = ORDERING_FUNCTION(100000);
    printf("%u\n", result);
    return result != 1000000;
}
