#include <limits.h>

/* Negating the lowest signed case value must not overflow the table-base
   computation or turn into an invalid/truncated assembler immediate. */
__attribute__((noinline)) int min_dense_switch(long long x) {
    switch (x) {
        case LLONG_MIN: return 11;
        case LLONG_MIN + 1: return 13;
        case LLONG_MIN + 2: return 17;
        case LLONG_MIN + 3: return 19;
        case LLONG_MIN + 4: return 23;
        default: return 7;
    }
}

static int expected(long long x) {
    if (x == LLONG_MIN) return 11;
    if (x == LLONG_MIN + 1) return 13;
    if (x == LLONG_MIN + 2) return 17;
    if (x == LLONG_MIN + 3) return 19;
    if (x == LLONG_MIN + 4) return 23;
    return 7;
}

int main(void) {
    const long long inputs[] = {
        0, LLONG_MAX, LLONG_MIN, LLONG_MIN + 1,
        LLONG_MIN + 2, LLONG_MIN + 3, LLONG_MIN + 4, LLONG_MIN + 5,
    };
    for (unsigned int i = 0; i < sizeof(inputs) / sizeof(inputs[0]); ++i) {
        if (min_dense_switch(inputs[i]) != expected(inputs[i])) {
            return (int)i + 1;
        }
    }
    return 0;
}
