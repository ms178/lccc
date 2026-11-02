extern unsigned pc32(unsigned);
extern unsigned pc64(unsigned long long);
extern unsigned pcl(unsigned long);
extern unsigned pcs32(int);
extern unsigned pcs64(long long);
extern unsigned long long pressure(unsigned long long, unsigned long long,
    unsigned long long, unsigned long long, unsigned long long, unsigned long long,
    unsigned long long, unsigned long long, unsigned long long, unsigned long long);
extern unsigned pc32_dirty(unsigned long long);
extern unsigned pcu8(unsigned char);
extern unsigned pcs8(signed char);
extern unsigned pcu16(unsigned short);
extern unsigned pcs16(short);
static unsigned ref(unsigned long long x) {
    static const unsigned char counts[16] = {0,1,1,2,1,2,2,3,1,2,2,3,2,3,3,4};
    unsigned result = 0;
    for (unsigned i=0; i<16; ++i) { result += counts[x & 15]; x >>= 4; }
    return result;
}
static int check(unsigned long long x) {
    unsigned r = ref(x), r32 = ref((unsigned)x);
    if (pcl(x)!=r || pc64(x)!=r || pc32(x)!=r32 || pcs64((long long)x)!=r || pcs32((int)x)!=r32) return 1;
    if (pc32_dirty(x | 0xffffffff00000000ULL) != r32) return 1;
    unsigned r8=ref(x & 255), r16=ref(x & 65535);
    if (pcu8(x)!=r8 || pcs8(x)!=r8+((x&128)?24:0)) return 1;
    if (pcu16(x)!=r16 || pcs16(x)!=r16+((x&32768)?16:0)) return 1;
    return pressure(x,2,3,4,5,6,7,8,9,10) != r+2+9+20+35+66+91+136+171+230;
}
int test(void) {
    if(check(0) || check(~0ULL)) return 1;
    for(unsigned i=0;i<64;++i) if(check(1ULL<<i) || check(~(1ULL<<i))) return 2;
    for(unsigned i=0;i<65536;++i) if(check(i) || check((unsigned long long)i<<32)) return 3;
    unsigned long long x=0x8123456789abcdefULL;
    for(unsigned i=0;i<4096;++i) { x^=x<<13; x^=x>>7; x^=x<<17; if(check(x)) return 4; }
    return 0;
}
