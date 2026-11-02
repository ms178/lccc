/* Regression: phi-web coalescing must not alias EXTERNAL feed values into a
 * loop-carried copy web.
 *
 * Bug class (found via zlib-ng maketrees, 2026-08): the slot-coalescing phi
 * web pass merged values like the last-iteration separator string address
 * (defined outside the loop) into the loop-carried web. The elided phi Copy
 * then left the shared web home holding the PREVIOUS iteration's value, and
 * reads of the "merged" value returned that stale value instead (the printf
 * separator came out as the previous iteration's "," instead of "\n};\n\n").
 *
 * Shape: two cross-copying loop-carried values (c1, c2 — the 209/210/207
 * pattern) plus an external constant fed into the web on the final iteration.
 * Any staleness in the web changes the checksums below. */
#include <stdio.h>
#include <string.h>

static const char *g_final = "FINAL";

static unsigned f(unsigned n) {
    const char *sep = ",";
    unsigned long acc = 0;
    const char *c1 = "";
    const char *c2 = "";
    for (unsigned i = 0; i <= n; i++) {
        const char *s;
        if (i == n)
            s = g_final; /* external feed on the last iteration */
        else
            s = sep;
        c1 = s;
        c2 = c1; /* second copy: cross-copying web members */
        acc += strlen(s);
        acc += (unsigned long)(unsigned char)c2[0];
    }
    return (unsigned)(acc + strlen(c1) + strlen(c2));
}

int main(void) {
    /* n=7: 7 iterations of "," (len 1 + ','=44 => 45 each... strlen(",")=1,
     * c2[0]=44) for i=0..6 => 7*45=315; final: strlen("FINAL")=5 + 'F'=70
     * => 75; tail strlen(c1)+strlen(c2) = 5+5 = 10. Total = 315+75+10 = 400 */
    if (f(7) != 400) {
        printf("bad f(7)=%u\n", f(7));
        return 1;
    }
    /* n=1: i=0: 1+44=45; final: 75; tail 10 => 130 */
    if (f(1) != 130) {
        printf("bad f(1)=%u\n", f(1));
        return 2;
    }
    /* n=100: 100*45 + 75 + 10 = 4585 */
    if (f(100) != 4585) {
        printf("bad f(100)=%u\n", f(100));
        return 3;
    }
    return 0;
}
