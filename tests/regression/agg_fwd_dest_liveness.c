/* Regression: forward_store_only_temporaries must verify the memcpy DEST's
 * old contents are dead in [first redirected use, copy). zlib-ng fold_1
 * register rotation: tmp=*c3; *c3=*c0; *c1=*c2; *c2=tmp — the snapshot
 * write was redirected into c2 while *c1=*c2 still read c2's OLD value
 * (CRC mismatch on every fold_copy+fold pair with unaligned lengths). */
#include <string.h>
#include <stdio.h>
typedef struct { unsigned long long a, b; } V;
static void __attribute__((noinline)) rot(V *c0, V *c1, V *c2, V *c3) {
    V tmp;                 /* snapshot alloca */
    memcpy(&tmp, c3, 16);  /* tmp = *c3 */
    memcpy(c3, c0, 16);    /* mutate */
    memcpy(c1, c2, 16);    /* READS c2's OLD value */
    memcpy(c2, &tmp, 16);  /* c2 = snapshot (the forwarding target) */
    c3->a ^= c3->b;
}
int main(void){
    V c0 = {1,10}, c1 = {2,20}, c2 = {3,30}, c3 = {4,40};
    rot(&c0, &c1, &c2, &c3);
    printf("%llu %llu %llu %llu\n", c1.a, c1.b, c2.a, c2.b);
    /* c1 must have OLD c2 (3,30); c2 must have old c3 (4,40) */
    return (c1.a == 3 && c1.b == 30 && c2.a == 4 && c2.b == 40) ? 0 : 1;
}
