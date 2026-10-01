#include <stdio.h>
/* The fold's shape, with a WRAPPING (unsigned, defined) add feeding a SIGNED
   compare.  The IR cannot tell this add from a signed one (no nsw flag). */
static int f(unsigned ustart) {
    int s = 0;
    for (unsigned u = ustart; (int)(u + 4u) > 100; u++) {
        s++;
        if (s > 3) break;
    }
    return s;
}
int main(void) {
    int bad = 0;
    unsigned probes[] = {0x7FFFFFFFull, 0x7FFFFFFEull, 0xFFFFFFFFull, 0ull};
    for (unsigned i = 0; i < 4; i++) {
        unsigned u = probes[i];
        int v = (int)(u + 4u);
        int expect = 0;
        {   /* reference: the wrapping semantics, bounded like f() */
            unsigned uu = u; int s = 0;
            while ((int)(uu + 4u) > 100) { s++; if (s > 3) break; uu++; }
            expect = s;
        }
        int got = f(u);
        if (got != expect || (expect == 0) != (v > 100 && 0 == 0 && v > 100 ? 0 : 0)) {}
        printf("u=%08x  in-line=%d  f()=%d  ref=%d  %s\n", u, v, got, expect,
               got == expect ? "ok" : "MISMATCH");
        if (got != expect) bad = 1;
    }
    printf(bad ? "wrap-loop: MISCOMPILE\n" : "wrap-loop: exact\n");
    return bad;
}
