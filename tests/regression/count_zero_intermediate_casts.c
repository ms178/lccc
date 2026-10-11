/* a9987d0 red-team: Select traversal dropped truncations between count-zero
 * stages. clz_trunc(2) was 30 instead of 31; ctz_trunc(0x10000) must be 31.
 * Native idiom controls and independent bit-at-a-time expectations included. */
extern int printf(const char *, ...);

#define CLZ(name, mid) \
__attribute__((noinline)) unsigned name(unsigned x) { \
    unsigned n=0; \
    if (x<=0xffffu) {n+=16; x<<=16;} \
    mid; \
    if (x<=0xffffffu) {n+=8; x<<=8;} \
    if (x<=0xfffffffu) {n+=4; x<<=4;} \
    if (x<=0x3fffffffu) {n+=2; x<<=2;} \
    if (x<=0x7fffffffu) n++; \
    return n; \
}
CLZ(clz_full, )
CLZ(clz_trunc, x=(unsigned short)x)

#define CTZ(name, mid) \
__attribute__((noinline)) unsigned name(unsigned long long x) { \
    unsigned n=0; \
    if (!(x&0xffffffffull)) {n+=32; x>>=32;} \
    mid; \
    if (!(x&0xffffull)) {n+=16; x>>=16;} \
    if (!(x&0xffull)) {n+=8; x>>=8;} \
    if (!(x&0xfull)) {n+=4; x>>=4;} \
    if (!(x&3ull)) {n+=2; x>>=2;} \
    if (!(x&1ull)) n++; \
    return n; \
}
CTZ(ctz_full, )
CTZ(ctz_trunc, x=(unsigned short)x)

__attribute__((noinline)) unsigned ref_clz(unsigned x) {
    unsigned n=0;
    while (n<31 && !(x&0x80000000u)) {n++; x<<=1;}
    return n;
}
__attribute__((noinline)) unsigned ref_ctz(unsigned long long x, unsigned limit) {
    unsigned n=0;
    while (n<limit && !(x&1ull)) {n++; x>>=1;}
    return n;
}
int main(void) {
    unsigned state=123456789u;
    for (unsigned i=0; i<4096; i++) {
        state=state*1664525u+1013904223u;
        unsigned x=i==0 ? 2u : i<34 ? (i==1 ? 0u : 1u<<(i-2)) : state;
        unsigned long long y=i<64 ? 1ull<<i : ((unsigned long long)x<<32)|state;
        if (i==64) y=0;
        unsigned base=(y&0xffffffffull) ? 0 : 32;
        unsigned long long low=(y>>base)&0xffffull;
        unsigned want_clz=x<=0xffffu ? 31 : 15;
        unsigned want_ctz=base+ref_ctz(low,31);
        if (clz_full(x)!=ref_clz(x) || clz_trunc(x)!=want_clz ||
            ctz_full(y)!=ref_ctz(y,63) || ctz_trunc(y)!=want_ctz) {
            printf("FAIL %08x %llx: clz=%u/%u ctz=%u/%u\n",x,y,
                   clz_trunc(x),want_clz,ctz_trunc(y),want_ctz);
            return 1;
        }
    }
    printf("PASS: 4096 count-zero inputs, four shapes\n");
    return 0;
}
