#include <stddef.h>
typedef unsigned char u8;
typedef unsigned long long u64;
static inline u64 read64(const void *p){ u64 v; __builtin_memcpy(&v,p,8); return v; }
u64 f(const u8* a, const u8* b, unsigned long n){
  u64 s=0; for(unsigned long i=0;i<n;i++){ u64 x=read64(a+i*8); u64 y=read64(b+i*8); s += x^y; } return s;
}
