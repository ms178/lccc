// Deterministic SPSC ring FIFO benchmark.
//
// Exercises masking, modulo-free wrap-around, dependent stores/loads, and branch
// structure common in network/parser buffers.  The sequence is fixed so all
// compilers must produce identical output.
#include <stdint.h>
#include <stddef.h>

enum { CAP = 1024, OPS = 20000 };
static uint32_t ring[CAP];

int main(void) {
    uint32_t head = 0, tail = 0, sum = 0;
    uint32_t seed = 1;
    for (int i = 0; i < OPS; ++i) {
        if (((head - tail) & (CAP - 1u)) != (CAP - 1u)) {
            seed = seed * 1664525u + 1013904223u;
            ring[head & (CAP - 1u)] = seed;
            head++;
        }
        if (head != tail) {
            sum += ring[tail & (CAP - 1u)];
            tail++;
        }
    }
    if (head != OPS || tail != OPS)
        return 1;
    return sum == 1920339856u ? 0 : 2;
}
