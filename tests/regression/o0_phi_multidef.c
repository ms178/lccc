/* O0 non-SSA multi-def regression: register allocation used a stale loop-carried c value. */
#include <stdint.h>
static volatile uint64_t observe;
static uint64_t rot(uint64_t x, unsigned n) { n &= 63u; return n ? ((x << n) | (x >> ((64u-n)&63u))) : x; }
static uint64_t mix(uint64_t a, uint64_t b, unsigned n) { a ^= rot(b + UINT64_C(0x9e3779b97f4a7c15), n); a *= UINT64_C(0xbf58476d1ce4e5b9); return a ^ (a >> 31); }
static uint64_t kernel(uint64_t seed, unsigned limit) {
  uint64_t a=UINT64_C(0x1e2feb89414c343c) ^ seed;
  uint64_t b=UINT64_C(0xc2ce6f447ed4d57b) + seed;
  uint64_t c=UINT64_C(0x78e510617311d8a3);
  uint64_t table[8];
  for (unsigned z=0; z<8; ++z) table[z] = mix(a+z,b^z,z);
  for (unsigned i=0; i<limit; ++i) {
    switch ((unsigned)((a ^ b ^ c ^ i) & 7u)) {
      case 0: { a = mix(a ^ UINT64_C(0x612e7696a6cecc1b), b + c, i); b ^= rot(c + UINT64_C(0x35bf992dc9e9c616), i); break; }
      case 1: { c = mix(c + UINT64_C(0x7ce42c8218072e8c), a ^ b, i+1u); a += rot(b ^ UINT64_C(0xe4b06ce60741c7a8), i); break; }
      case 2: { b = mix(b + UINT64_C(0x63ca828dd5f4b3b2), c, i+2u); c ^= rot(a + UINT64_C(0x9b810e766ec9d286), i); break; }
      case 3: { a ^= mix(b, UINT64_C(0xc4647159c324c985), i); c += mix(a, UINT64_C(0xb2221a58008a05a6), i+3u); break; }
      case 4: { a = mix(a ^ UINT64_C(0x442e3d437204e52d), b + c, i); b ^= rot(c + UINT64_C(0xcd447e35b8b6d8fe), i); break; }
      case 5: { c = mix(c + UINT64_C(0x9755d4c13a902931), a ^ b, i+1u); a += rot(b ^ UINT64_C(0x1a2b8f1ff1fd42a2), i); break; }
      case 6: { b = mix(b + UINT64_C(0x51431193e6c3f339), c, i+2u); c ^= rot(a + UINT64_C(0x05b6e6e307d4bedc), i); break; }
      case 7: { a ^= mix(b, UINT64_C(0xa648a7dd06839eb9), i); c += mix(a, UINT64_C(0x025b413f8a9a021e), i+3u); break; }
    }
    if ((a + i) & 1u) {
      uint64_t old = b++;
      a ^= mix(old, c, i);
    } else {
      uint64_t old = c--;
      b ^= mix(old, a, i+1u);
    }
    for (unsigned j=0; j<3; ++j) {
      unsigned idx=(unsigned)((a+b+c+j)&7u);
      if ((table[idx] ^ a) & 1u) { a += table[idx]; continue; }
      b ^= table[idx]; c += (a ^ b) + j;
    }
    observe = a ^ b ^ c;
    if ((observe & 15u) == 3u) { c ^= observe; }
  }
  return a ^ rot(b,17) ^ rot(c,39) ^ observe;
}
int main(void) {
  uint64_t x = kernel(UINT64_C(0xe1988ad9f06c144a), 7u);
  if (x != UINT64_C(0xf6ef0ff3713b837c)) return 1;
  uint64_t y = kernel(x ^ UINT64_C(0xafbd67f9619699cf), 4u);
  if (y != UINT64_C(0xe7079f1780a20806)) return 2;
  return 0;
}
