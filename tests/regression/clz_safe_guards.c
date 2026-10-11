/* PR791: safe portable CLZ may lose OR1 only with a width-correct proof.
 * In particular a nonzero 64-bit guard says nothing about its low 32 bits. */
extern int printf(const char *, ...);
static volatile unsigned observed;
#define TREE \
    unsigned n=0; \
    if (x<=0xffffu) {n+=16; x<<=16;} \
    if (x<=0xffffffu) {n+=8; x<<=8;} \
    if (x<=0xfffffffu) {n+=4; x<<=4;} \
    if (x<=0x3fffffffu) {n+=2; x<<=2;} \
    if (x<=0x7fffffffu) n++; \
    return n
#define FN __attribute__((noinline)) unsigned
FN clz_guard32(unsigned x) { if (!x) return 32; TREE; }
FN clz_guard31(unsigned x) { if (!x) return 31; TREE; }
FN clz_guard_live(unsigned x) { if (!x) return 32; observed=x; TREE; }
FN clz_guard_narrow(unsigned x) { if (!(unsigned char)x) return 32; TREE; }
FN clz_guard_wide(unsigned long long a) { if (!a) return 32; unsigned x=(unsigned)a; TREE; }
FN clz_guard_same_width(unsigned x) { if (!(int)x) return 32; TREE; }
FN clz_guard_signed_widen(unsigned x) { if (!(long long)(int)x) return 32; TREE; }
FN clz_guard_zero_widen(unsigned x) { if (!(unsigned long long)x) return 32; TREE; }
/* CTZ's high-bit safety OR obeys the same dominance rule. This guard
 * models Linux find_bit, where retaining the OR would regress codegen. */
FN ctz_guard64(unsigned long long x) {
    if (!x) return 64;
    unsigned n=0;
    if (!(x&0xffffffffull)) {n+=32; x>>=32;}
    if (!(x&0xffffull)) {n+=16; x>>=16;}
    if (!(x&0xffull)) {n+=8; x>>=8;}
    if (!(x&0xfull)) {n+=4; x>>=4;}
    if (!(x&3ull)) {n+=2; x>>=2;}
    if (!(x&1ull)) n++;
    return n;
}
static unsigned ref_trailing(unsigned long long x) {
    unsigned n=0; while(n<64 && !(x&1ull)) {n++; x>>=1;} return n;
}
static unsigned ref(unsigned x) {
    unsigned n=0; while (n<31 && !(x&0x80000000u)) { n++; x<<=1; } return n;
}
int main(void) {
    unsigned state=0xabcdef01u;
    for (unsigned i=0;i<4096;i++) {
        state=state*1664525u+1013904223u;
        unsigned x=i==0 ? 0 : i<33 ? 1u<<(i-1) : state;
        unsigned expected=x ? ref(x) : 32;
        unsigned long long wide=((unsigned long long)(i&1)<<32)|x;
        if (clz_guard32(x)!=expected || clz_guard31(x)!=ref(x) ||
            clz_guard_live(x)!=expected || clz_guard_same_width(x)!=expected ||
            clz_guard_signed_widen(x)!=expected || clz_guard_zero_widen(x)!=expected ||
            clz_guard_narrow(x)!=((unsigned char)x ? ref(x) : 32) ||
            clz_guard_wide(wide)!=(wide ? ref(x) : 32) || ctz_guard64(wide)!=ref_trailing(wide)) {
            printf("FAIL clz guards %u %08x\n",i,x); return 1;
        }
    }
    if (clz_guard_wide(0x100000000ull)!=31 || clz_guard_narrow(0x100)!=32 ||
        ctz_guard64(1ull<<63)!=63 || ctz_guard64(0)!=64) return 2;
    printf("PASS: 4096 CLZ guard inputs, eight CLZ shapes, CTZ guard, and cast boundary witnesses\n");
    return 0;
}
