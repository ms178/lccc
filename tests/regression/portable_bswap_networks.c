typedef unsigned long long U64;
__attribute__((noinline)) U64 bytes64(U64 x) {
    return ((x & 0x00000000000000ffULL) << 56) |
           ((x & 0x000000000000ff00ULL) << 40) |
           ((x & 0x0000000000ff0000ULL) << 24) |
           ((x & 0x00000000ff000000ULL) << 8) |
           ((x & 0x000000ff00000000ULL) >> 8) |
           ((x & 0x0000ff0000000000ULL) >> 24) |
           ((x & 0x00ff000000000000ULL) >> 40) |
           ((x & 0xff00000000000000ULL) >> 56);
}
__attribute__((noinline)) U64 xor64(U64 x) {
    return ((x & 0x00000000000000ffULL) << 56) ^
           ((x & 0x000000000000ff00ULL) << 40) ^
           ((x & 0x0000000000ff0000ULL) << 24) ^
           ((x & 0x00000000ff000000ULL) << 8) ^
           ((x & 0x000000ff00000000ULL) >> 8) ^
           ((x & 0x0000ff0000000000ULL) >> 24) ^
           ((x & 0x00ff000000000000ULL) >> 40) ^
           ((x & 0xff00000000000000ULL) >> 56);
}
__attribute__((noinline)) U64 stages64(U64 x) {
 x = ((x >> 8) & 0x00ff00ff00ff00ffULL) | ((x & 0x00ff00ff00ff00ffULL) << 8);
 x = ((x >> 16) & 0x0000ffff0000ffffULL) | ((x & 0x0000ffff0000ffffULL) << 16);
 return (x >> 32) | (x << 32);
}
__attribute__((noinline)) unsigned bytes32(unsigned x) {
 return ((x & 255U) << 24) | ((x & 65280U) << 8) |
        ((x & 16711680U) >> 8) | ((x & 4278190080U) >> 24);
}
__attribute__((noinline)) unsigned stages32(unsigned x) {
 x = ((x >> 8) & 0x00ff00ffU) | ((x & 0x00ff00ffU) << 8);
 return (x >> 16) | (x << 16);
}
/* Not a permutation: one input byte is deliberately absent. */
__attribute__((noinline)) U64 missing_byte(U64 x) {
 return ((x & 0xffULL) << 56) | ((x & 0xff00ULL) << 40) |
        ((x & 0xff0000ULL) << 24) | ((x & 0xff000000ULL) << 8) |
        ((x & 0xff00000000ULL) >> 8) | ((x & 0xff0000000000ULL) >> 24) |
        ((x & 0xff00000000000000ULL) >> 56);
}
#ifndef BYTE_SWAP_KERNELS_ONLY
extern int puts(const char *);
static U64 reference(U64 x, unsigned bytes) {
 U64 result=0;
 for (unsigned i=0;i<bytes;++i) { result=(result<<8)|(x&255); x>>=8; }
 return result;
}
static int check(U64 x) {
 U64 want=reference(x,8);
 unsigned narrow=(unsigned)reference((unsigned)x,4);
 return bytes64(x)!=want || xor64(x)!=want || stages64(x)!=want || bytes32((unsigned)x)!=narrow ||
        stages32((unsigned)x)!=narrow || missing_byte(x)!=(want & ~(255ULL<<8));
}
int main(void) {
 for(unsigned lane=0;lane<8;++lane)
  for(unsigned byte=0;byte<256;++byte)
   if(check((U64)byte<<(8*lane))) return 1;
 U64 x=0x123456789abcdef0ULL;
 for(unsigned i=0;i<65536;++i) {
  x^=x<<13; x^=x>>7; x^=x<<17;
  if(check(x)) return 2;
 }
 puts("portable byte-swap networks: PASS (67584 inputs, six functions)");
 return 0;
}
#endif
