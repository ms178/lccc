/* End-to-end asm emission, including implicit string lengths/results.
 * This is a semantics witness, not a timing benchmark. SSE4a remains an
 * assembly-only matrix on hosts without that AMD extension. */
#include <stdint.h>
#include <stdio.h>
typedef short v8s __attribute__((vector_size(16)));
typedef unsigned char v16b __attribute__((vector_size(16)));
union words { v8s v; short s[8]; };
union bytes { v16b v; unsigned char b[16]; uint64_t q[2]; };
static int clamp(int x) { return x > 32767 ? 32767 : x < -32768 ? -32768 : x; }
int main(void) {
    union words a, b, r;
    short av[8] = {32767,32767,-32768,-32768,100,-200,7,9};
    short bv[8] = {-32768,32767,32767,-32768,1,2,-7,-9};
    for (int i=0;i<8;i++) { a.s[i]=av[i]; b.s[i]=bv[i]; }
    r.v=a.v;
    __asm__("phaddsw %1,%0" : "+x"(r.v) : "x"(b.v));
    for (int i=0;i<4;i++) {
        if (r.s[i]!=clamp((int)av[2*i]+av[2*i+1]) ||
            r.s[i+4]!=clamp((int)bv[2*i]+bv[2*i+1])) return 1;
    }
    r.v=a.v;
    __asm__("phsubsw %1,%0" : "+x"(r.v) : "x"(b.v));
    for (int i=0;i<4;i++) {
        if (r.s[i]!=clamp((int)av[2*i]-av[2*i+1]) ||
            r.s[i+4]!=clamp((int)bv[2*i]-bv[2*i+1])) return 2;
    }
    union bytes x, y, mask;
    for (int i=0;i<16;i++) { x.b[i]=0; y.b[i]=0; }
    x.b[0]='a'; y.b[0]='x'; y.b[1]='x'; y.b[2]='a';
    int index;
    __asm__("pcmpistri $0,%2,%1" : "=c"(index) : "x"(x.v), "xm"(y.v) : "cc");
    if (index!=2) return 3;
    __asm__("pcmpistrm $0,%2,%1\n\tmovdqa %%xmm0,%0" : "=x"(mask.v) : "x"(x.v), "xm"(y.v) : "xmm0", "cc");
    if (mask.q[0]!=4 || mask.q[1]!=0) return 4;
    __asm__("pcmpestri $0,%4,%3" : "=c"(index) : "a"(-1), "d"(3), "x"(x.v), "xm"(y.v) : "cc");
    if (index!=2) return 5;
    __asm__("pcmpestrm $0,%4,%3\n\tmovdqa %%xmm0,%0" : "=x"(mask.v) : "a"(-1), "d"(3), "x"(x.v), "xm"(y.v) : "xmm0", "cc");
    if (mask.q[0]!=4 || mask.q[1]!=0) return 6;
    puts("legacy saturated horizontal and implicit/explicit string results: PASS");
    return 0;
}
