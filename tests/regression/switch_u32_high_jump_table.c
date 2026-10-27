#include <stdint.h>

__attribute__((noinline)) unsigned int high_dense_switch(unsigned int x) {
    switch (x) {
        case UINT32_MAX - 3: return 11;
        case UINT32_MAX - 2: return 13;
        case UINT32_MAX - 1: return 17;
        case UINT32_MAX: return 19;
        default: return 7;
    }
}

static unsigned int expected(unsigned int x) {
    if (x == UINT32_MAX - 3) return 11;
    if (x == UINT32_MAX - 2) return 13;
    if (x == UINT32_MAX - 1) return 17;
    if (x == UINT32_MAX) return 19;
    return 7;
}

int main(void) {
    const uint32_t inputs[] = {
        0, 1, UINT32_C(0x7fffffff), UINT32_C(0x80000000),
        UINT32_MAX - 4, UINT32_MAX - 3, UINT32_MAX - 2,
        UINT32_MAX - 1, UINT32_MAX,
    };
    for (unsigned int i = 0; i < sizeof(inputs) / sizeof(inputs[0]); ++i) {
        if (high_dense_switch(inputs[i]) != expected(inputs[i])) {
            return (int)i + 1;
        }
    }
    return 0;
}
