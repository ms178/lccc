/* i686 `rep movsb` copies: the destination and source addresses are loaded
 * into %edi/%esi as a parallel move. Loading the destination first
 * unconditionally copied the destination onto itself whenever the source
 * pointer was homed in %edi (here: g's `*s = *d2`, destination homed in
 * %esi and source in %edi, a true exchange). Four pointers live across
 * copies too large for the straight-line window occupy every callee-saved
 * register; a wrapper re-runs the file at -Os, where the size rule sends
 * small copies down the same path. */
#include <stdio.h>
#include <string.h>
struct big { unsigned char b[300]; };
#define NI __attribute__((noinline))
NI void f(struct big *d, struct big *s, struct big *d2, struct big *d3) { *d = *s; *d2 = *s; *d3 = *s; *d = *d3; }
NI void g(struct big *d, struct big *s, struct big *d2, struct big *d3) { *d3 = *d; *s = *d2; *d = *s; *d2 = *d3; }
int main(void) {
    static struct big a, b, c, e; int bad = 0;
    for (int i = 0; i < 300; i++) b.b[i] = (unsigned char)(i * 7 + 1);
    f(&a, &b, &c, &e);
    bad |= memcmp(&a, &b, 300) | memcmp(&c, &b, 300) | memcmp(&e, &b, 300);
    struct big x, y, z, w;
    for (int i = 0; i < 300; i++) { x.b[i] = (unsigned char)i; y.b[i] = 0; z.b[i] = (unsigned char)(255 - i); w.b[i] = 9; }
    g(&x, &y, &z, &w);
    for (int i = 0; i < 300; i++) bad |= (x.b[i] != (unsigned char)(255 - i)) | (w.b[i] != (unsigned char)i) | (z.b[i] != (unsigned char)i) | (y.b[i] != (unsigned char)(255 - i));
    printf("%s\n", bad ? "BAD" : "ok"); return bad != 0;
}
