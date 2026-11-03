typedef unsigned u32; typedef unsigned long long u64; typedef int i32;
u32 np2(u32 x)      { return 1u << (32 - __builtin_clz(x - 1)); }
u32 np2b(u32 x)     { return 1u << (32 - __builtin_clz(x)); }
u64 np2ll(u64 x)    { return 1ull << (64 - __builtin_clzll(x - 1)); }
u32 np2_guard(u32 x){ return x <= 1 ? 1u : (1u << (32 - __builtin_clz(x - 1))); }
i32 fls(u32 x)      { return x ? 32 - __builtin_clz(x) : 0; }
u32 highbit(u32 x)  { return 31u - __builtin_clz(x); }
u32 msb_pow2(u32 x) { return 1u << (31 - __builtin_clz(x)); }
