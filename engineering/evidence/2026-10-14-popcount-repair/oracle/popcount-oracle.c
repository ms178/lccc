unsigned pc32(unsigned x) { return __builtin_popcount(x); }
unsigned pc64(unsigned long long x) { return __builtin_popcountll(x); }
unsigned sw32(unsigned x) {
 x -= (x >> 1) & 0x55555555U;
 x = (x & 0x33333333U) + ((x >> 2) & 0x33333333U);
 x = (x + (x >> 4)) & 0x0f0f0f0fU;
 return (x * 0x01010101U) >> 24;
}
unsigned sw64(unsigned long long x) {
 x -= (x >> 1) & 0x5555555555555555ULL;
 x = (x & 0x3333333333333333ULL) + ((x >> 2) & 0x3333333333333333ULL);
 x = (x + (x >> 4)) & 0x0f0f0f0f0f0f0f0fULL;
 return (x * 0x0101010101010101ULL) >> 56;
}

static unsigned ref(unsigned long long x) { unsigned n=0; while(x) {n += x&1; x >>=1;} return n; }
int main(void) {
 unsigned long long x=0x8123456789abcdefULL;
 for(unsigned i=0;i<4096;i++) {
  x ^= x<<13; x^=x>>7; x^=x<<17;
  if(sw64(x)!=ref(x) || pc64(x)!=ref(x) || sw32((unsigned)x)!=ref((unsigned)x) || pc32((unsigned)x)!=ref((unsigned)x)) return 1;
 }
 for(unsigned i=0;i<64;i++) { x=1ULL<<i; if(sw64(x)!=1 || sw64(~x)!=63) return 2; }
 return sw64(~0ULL)!=64 || sw32(~0U)!=32 || sw64(0)!=0 || sw32(0)!=0;
}
