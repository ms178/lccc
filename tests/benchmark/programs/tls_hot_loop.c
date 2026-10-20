#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

__thread volatile uint64_t tls_hot_value = UINT64_C(0x9e3779b97f4a7c15);

__attribute__((noinline)) uint64_t tls_hot_loop(uint64_t iterations) {
    uint64_t sum = 0;
    for (uint64_t i = 0; i < iterations; ++i)
        sum += tls_hot_value;
    return sum;
}

int main(int argc, char **argv) {
    uint64_t iterations = argc > 1 ? strtoull(argv[1], 0, 0) : 1000000;
    uint64_t result = tls_hot_loop(iterations);
    printf("%016llx\n", (unsigned long long)result);
    return result != iterations * tls_hot_value;
}
