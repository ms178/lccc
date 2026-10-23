/* Host for gd_acc.c: defines ext_tls and checks every result. */
#include <stdio.h>
__thread long ext_tls = 5;
long w1(long *, long);
long w2(long, long);
long w3(long, long, long);
int main(void) {
    long q = 7;
    int bad = 0;
    long r1 = w1(&q, 3); /* v = 21: 21 + 5 + 63 */
    if (r1 != 89) { printf("w1 = %ld, want 89\n", r1); bad = 1; }
    long r2 = w2(0x1234, 0x0ff0); /* v = 0x1dc4, stored to ext_tls */
    if (r2 != 0x1dc5 || ext_tls != 0x1dc4) {
        printf("w2 = %ld, ext_tls = %#lx; want 7621, 0x1dc4\n", r2, ext_tls);
        bad = 1;
    }
    long r3 = w3(2, 3, 4); /* v = 5: 20 + 0x1dc4 + 5 */
    if (r3 != 25 + 0x1dc4) { printf("w3 = %ld, want %d\n", r3, 25 + 0x1dc4); bad = 1; }
    return bad;
}
