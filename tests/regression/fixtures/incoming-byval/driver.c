#include "common.h"
#include <stdint.h>
long aligned_address(const void *p, unsigned a) { return ((uintptr_t)p & (a-1)) == 0; }
long adjust(struct Big *s, const struct Big *original) {
    if (s == original || s->a != original->a || s->b != original->b || s->c != original->c)
        return -3000;
    s->b += 11;
    return 5;
}
__attribute__((noinline)) long read_big(struct Big *s) {
    return s->a+s->b+s->c;
}
long consume_wide(struct Wide s) {
    if ((uintptr_t)&s & 63) return -4000;
    return s.a[0] + s.a[7];
}
int main(void) {
    for (long k = -20; k <= 20; ++k) {
        struct Big s = {k, 2*k+1, 3*k-1};
        struct Small t = {k, k+3};
        struct Packed p = {3, k};
        struct Wide w = {{k,0,0,0,0,0,0,k+2}};
        /* Volatile forces the stack-overflow binary128 argument through its
           16-byte SSA slot; the constant call below covers immediate images. */
        volatile __float128 xf = -2*k+1;
        long sum = 6*k;
        if (sum_big(s) != sum) return 1;
        if (tail_big(s) != sum) return 13;
        if (mutate_big(s, &s) != sum+16) return 2;
        if (s.a != k || s.b != 2*k+1 || s.c != 3*k-1) return 3;
        if (dynamic_big(s, (unsigned)(k+20)) != sum+7) return 4;
        if (realign_big(s, w) != sum+2*k+2) return 5;
        if (aligned_f128(1,2,3,4,5,6,7,1,2,3,4,5,6,7,8,9,10) != 83) return 12;
        if (aligned_f128(1,2,3,4,5,6,7,1,2,3,4,5,6,7,8,xf,10) != 75-2*k) return 14;
        if (aligned_object(1,2,3,4,5,6,7,s,8) != 36+sum) return 6;
        if (variadic_big(s, 13L, s) != 2*sum+13) return 7;
        if (exhausted_small(1,2,3,4,5,6,t) != 2*k+24) return 8;
        if (packed_value(p) != k+3) return 9;
        if (volatile_big(s) != sum+7) return 11;
        struct Big r = return_big(s);
        if (r.a != s.a || r.b != s.b || r.c != s.c+9) return 10;
    }
    return 0;
}
