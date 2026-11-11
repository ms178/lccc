#include <stdio.h>
typedef float v4 __attribute__((vector_size(16)));
typedef float v8 __attribute__((vector_size(32)));
extern v4 ret4(v4);
extern v8 ret8(v8);
extern v4 fwd4(v4);
extern v8 fwd8(v8);

int main(void) {
    int bad = 0;
    v4 i4 = {1, 2, 3, 4};
    v8 i8 = {1, 2, 3, 4, 5, 6, 7, 8};
    const float w4[4] = {2, 4, 6, 8};
    const float w8[8] = {2, 4, 6, 8, 10, 12, 14, 16};

    v4 r4 = ret4(i4), f4 = fwd4(i4);
    v8 r8 = ret8(i8), f8 = fwd8(i8);
    for (int i = 0; i < 4; i++) {
        if (r4[i] != w4[i]) { printf("ret4 lane%d %g want %g\n", i, r4[i], w4[i]); bad++; }
        if (f4[i] != w4[i]) { printf("fwd4 lane%d %g want %g\n", i, f4[i], w4[i]); bad++; }
    }
    for (int i = 0; i < 8; i++) {
        if (r8[i] != w8[i]) { printf("ret8 lane%d %g want %g\n", i, r8[i], w8[i]); bad++; }
        if (f8[i] != w8[i]) { printf("fwd8 lane%d %g want %g\n", i, f8[i], w8[i]); bad++; }
    }
    if (bad) { printf("FAIL: %d lane(s) corrupted across a vector return\n", bad); return 1; }
    printf("PASS: every lane survives a vector return at 128 and 256 bits\n");
    return 0;
}
