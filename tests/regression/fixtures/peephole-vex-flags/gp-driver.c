#include <stdint.h>
#include <stdio.h>
extern uint64_t original_gp_product(uint64_t, uint64_t, uint64_t, uint64_t, uint64_t);
extern uint64_t rewritten_gp_product(uint64_t, uint64_t, uint64_t, uint64_t, uint64_t);
int main(void) {
    uint64_t a = original_gp_product(UINT64_C(0x8000000000000000), 1, 0, 0, 4);
    uint64_t b = rewritten_gp_product(UINT64_C(0x8000000000000000), 1, 0, 0, 4);
    printf("implicit GP product: original=%llu illegal=%llu\n",
           (unsigned long long)a, (unsigned long long)b);
    return a != 2 || b != 0;
}
