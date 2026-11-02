#include <stdint.h>
#include <stdio.h>
#include <string.h>

extern void shuffle_symbolic_load(uint32_t *);
extern void shift_symbolic_load(uint32_t *);
extern void extract_symbolic_store(void);
extern unsigned char rip_output[17];

int main(void) {
    if (!__builtin_cpu_supports("avx512f") || !__builtin_cpu_supports("avx512vl")) {
        puts("SKIP: AVX512F and AVX512VL required");
        return 77;
    }
    uint32_t actual[4];
    const uint32_t input[4] = {1, UINT32_MAX, UINT32_C(0x80000000), UINT32_C(0x12345678)};
    int failed = 0;
    shift_symbolic_load(actual);
    for (unsigned i = 0; i < 4; ++i) {
        if (actual[i] != (input[i] << 3)) {
            fprintf(stderr, "RIP-relative immediate load: lane %u got %08x\n", i, actual[i]);
            failed = 1;
            break;
        }
    }
    extract_symbolic_store();
    const uint32_t expected[4] = {1, 1, 1, 1};
    if (memcmp(rip_output, expected, sizeof(expected)) || rip_output[16] != 0xa5) {
        fputs("RIP-relative immediate store changed the wrong bytes\n", stderr);
        failed = 1;
    }
    shuffle_symbolic_load(actual);
    for (unsigned i = 0; i < 4; ++i) {
        if (actual[i] != 1) {
            fprintf(stderr, "RIP-relative legacy immediate load: lane %u got %08x\n", i, actual[i]);
            failed = 1;
            break;
        }
    }
    if (!failed) puts("RIP-relative immediate load/store: PASS");
    return failed;
}
