#include <stdio.h>
static unsigned long long probes[] = {
    0x7FFFFFFEull, 0xFFFFFFFEull, 0x7FFFFFFFull, 0x80000000ull, 0x00000000ull,
};
int f(unsigned u) {
    int v = (int)(u + 4u);       /* unsigned add: WRAPS, defined */
    return v < 100;              /* signed compare: the fold's shape */
}
int main(void) {
    int bad = 0;
    for (unsigned i = 0; i < sizeof probes / sizeof *probes; i++) {
        unsigned u = (unsigned)probes[i];
        int v = (int)(u + 4u);
        int r = f(u);
        if ((v < 100) != r) { printf("MISMATCH u=%08x inline=%d f=%d\n", u, v < 100, r); bad = 1; }
    }
    printf(bad ? "wrap-affine: MISCOMPILE\n" : "wrap-affine: exact\n");
    return bad;
}
