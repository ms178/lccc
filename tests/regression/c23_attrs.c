/* v4 regression: C23 [[fallthrough]] + [[maybe_unused]] statement attributes
 * (v1 glibc cpu-features bug class), plus _Static_assert and _Alignas. */
#include <stdio.h>
#include <stddef.h>

_Static_assert(sizeof(int) == 4, "int must be 4 bytes");
_Static_assert(offsetof(struct { char c; int i; }, i) == 4, "offset");

struct Aligned32 {
    char c;
} __attribute__((aligned(32)));
_Alignas(64) char big_buf[16];

static int f(int x) {
    switch (x) {
    case 1:
        x += 10;
        [[fallthrough]];
    case 2:
        x += 100;
        break;
    default:
        x = -1;
    }
    return x;
}

static int g(void) {
    int y = 1;
    [[maybe_unused]] int unused_var = 42;   /* must not warn/error */
    (void)unused_var;
    [[maybe_unused]] int z = 5;
    y += z;
    return y;
}

int main(void) {
    if (f(1) != 111) return 1;
    if (f(2) != 102) return 2;
    if (f(3) != -1) return 3;
    if (g() != 6) return 4;
    if (sizeof(struct Aligned32) != 32) return 5;
    if ((unsigned long)big_buf % 64 != 0) return 6;   /* _Alignas honored */
    /* [[nodiscard]]-style warnings are suppressed; nothing to check beyond compile */
    printf("OK c23_attrs\n");
    return 0;
}
