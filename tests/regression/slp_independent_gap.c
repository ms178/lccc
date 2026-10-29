/* A same-stream load two words behind a two-word store is not necessarily a
 * loop recurrence. With a four-word induction step, the loaded words lie in
 * the untouched gap between successive stores. Keep this profitable pair
 * eligible for BB-SLP while protecting a real distance-one recurrence. */
#include <stdint.h>
#include <stdio.h>

#define N 128

__attribute__((noinline)) void gap_pairs(uint32_t *a, int n) {
    for (int i = 2; i + 1 < n; i += 4) {
        a[i] = (a[i - 2] + 0x9e3779b9u) ^ (a[i - 2] >> 7);
        a[i + 1] = (a[i - 1] + 0x9e3779b9u) ^ (a[i - 1] >> 7);
    }
}

int main(void) {
    uint32_t actual[N], expected[N];
    for (int i = 0; i < N; ++i)
        actual[i] = expected[i] = (uint32_t)i * 2654435761u;
    for (int i = 2; i + 1 < N; i += 4) {
        expected[i] = (expected[i - 2] + 0x9e3779b9u) ^ (expected[i - 2] >> 7);
        expected[i + 1] = (expected[i - 1] + 0x9e3779b9u) ^ (expected[i - 1] >> 7);
    }
    gap_pairs(actual, N);
    for (int i = 0; i < N; ++i)
        if (actual[i] != expected[i])
            return 1;
    puts("slp independent gap: ok");
    return 0;
}
