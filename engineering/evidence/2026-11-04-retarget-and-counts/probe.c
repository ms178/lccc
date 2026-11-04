typedef unsigned u32; typedef unsigned long long u64; typedef int i32;
u32 np2(u32 x)      { return 1u << (32 - __builtin_clz(x - 1)); }
u64 np2ll(u64 x)    { return 1ull << (64 - __builtin_clzll(x - 1)); }
u32 highbit(u32 x)  { return 31u - __builtin_clz(x); }
i32 fls(u32 x)      { return x ? 32 - __builtin_clz(x) : 0; }
u32 clz32(u32 x)    { return __builtin_clz(x); }
u32 ctz32(u32 x)    { return __builtin_ctz(x); }
u32 clz64(u64 x)    { return __builtin_clzll(x); }
u32 ctz64(u64 x)    { return __builtin_ctzll(x); }
u32 pc32(u32 x)     { return __builtin_popcount(x); }
u32 pc64(u64 x)     { return __builtin_popcountll(x); }
u64 bsw64(u64 x)    { return __builtin_bswap64(x); }
u32 bsw32(u32 x)    { return __builtin_bswap32(x); }
u32 ffs32(u32 x)    { return __builtin_ffs(x); }
u32 rol32(u32 x, u32 r) { return (x << (r & 31)) | (x >> (32 - (r & 31))); }
u64 shl64v(u64 x, u32 r) { return x << (r & 63); }
