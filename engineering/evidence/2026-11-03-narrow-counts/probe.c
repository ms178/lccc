typedef unsigned char u8; typedef unsigned short u16; typedef unsigned u32; typedef unsigned long long u64;
int  pc_u8(u8 x)    { return __builtin_popcount(x); }
int  pc_u16(u16 x)  { return __builtin_popcount(x); }
int  pc32(u32 x)    { return __builtin_popcount(x); }
int  pc64(u64 x)    { return __builtin_popcountll(x); }
int  par_u8(u8 x)   { return __builtin_parity(x); }
int  par_u16(u16 x) { return __builtin_parity(x); }
int  par32(u32 x)   { return __builtin_parity(x); }
int  par64(u64 x)   { return __builtin_parityll(x); }
u32  np2(u32 x)     { return 1u << (32 - __builtin_clz(x - 1)); }
u32  np2b(u32 x)    { return 1u << (32 - __builtin_clz(x)); }
