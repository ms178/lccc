typedef unsigned char u8;
typedef signed char s8;
typedef unsigned short u16;
typedef short s16;
typedef unsigned u32;
typedef int s32;
typedef unsigned long long u64;
typedef long long s64;
static u64 h = 1469598103934665603ull;
static void mix(u64 v){ for(int i=0;i<8;i++){ h ^= (v>>(8*i)) & 255u; h *= 1099511628211ull; } }
__attribute__((noinline)) int set8_neg(s8 x,u32 n){ s8 y=(s8)(x | (1u<<n)); return y < 0; }
__attribute__((noinline)) int clr8_neg(s8 x,u32 n){ s8 y=(s8)(x & ~(1u<<n)); return y < 0; }
__attribute__((noinline)) s64 set8_wide(s8 x,u32 n){ return (s8)(x | (1u<<n)); }
__attribute__((noinline)) u64 clr8_wide(u8 x,u32 n){ return (u8)(x & ~(1u<<n)); }
__attribute__((noinline)) int set16_neg(s16 x,u32 n){ s16 y=(s16)(x | (1u<<n)); return y < 0; }
__attribute__((noinline)) int clr16_neg(s16 x,u32 n){ s16 y=(s16)(x & ~(1u<<n)); return y < 0; }
__attribute__((noinline)) s64 set16_wide(s16 x,u32 n){ return (s16)(x | (1u<<n)); }
__attribute__((noinline)) u64 clr16_wide(u16 x,u32 n){ return (u16)(x & ~(1u<<n)); }
__attribute__((noinline)) int set32_neg(s32 x,u32 n){ u32 y=(u32)x | (1u<<n); return (s32)y < 0; }
__attribute__((noinline)) int clr32_neg(s32 x,u32 n){ u32 y=(u32)x & ~(1u<<n); return (s32)y < 0; }
__attribute__((noinline)) s64 set32_wide(s32 x,u32 n){ return (s32)((u32)x | (1u<<n)); }
__attribute__((noinline)) u64 clr32_wide(u32 x,u32 n){ return x & ~(1u<<n); }
__attribute__((noinline)) int set64_neg(s64 x,u64 n){ u64 y=(u64)x | (1ull<<n); return (s64)y < 0; }
__attribute__((noinline)) int clr64_neg(s64 x,u64 n){ u64 y=(u64)x & ~(1ull<<n); return (s64)y < 0; }
__attribute__((noinline)) u64 set64(u64 x,u64 n){ return x | (1ull<<n); }
__attribute__((noinline)) u64 clr64(u64 x,u64 n){ return x & ~(1ull<<n); }
u64 bit_width_checksum(void){
  static const u32 pat[]={0u,0xffffffffu,0x80000000u,0x7fffffffu,0xaaaaaaaau,0x55555555u,0xdeadbeefu,1u};
  for(unsigned p=0;p<8;p++){
    u32 x=pat[p];
    for(u32 n=0;n<8;n++){
      mix(set8_neg((s8)x,n)); mix(clr8_neg((s8)x,n));
      mix((u64)set8_wide((s8)x,n)); mix(clr8_wide((u8)x,n));
    }
    for(u32 n=0;n<16;n++){
      mix(set16_neg((s16)x,n)); mix(clr16_neg((s16)x,n));
      mix((u64)set16_wide((s16)x,n)); mix(clr16_wide((u16)x,n));
    }
    for(u32 n=0;n<32;n++){
      mix(set32_neg((s32)x,n)); mix(clr32_neg((s32)x,n));
      mix((u64)set32_wide((s32)x,n)); mix(clr32_wide(x,n));
    }
    u64 xx=((u64)x<<32)|pat[(p+3)&7];
    for(u64 n=0;n<64;n++){
      mix(set64_neg((s64)xx,n)); mix(clr64_neg((s64)xx,n));
      mix(set64(xx,n)); mix(clr64(xx,n));
    }
  }
  return h;
}

__attribute__((noinline)) int tog8_neg(s8 x,u32 n){ s8 y=(s8)(x ^ (1u<<n)); return y < 0; }
__attribute__((noinline)) s64 tog8_wide_s(s8 x,u32 n){ return (s8)(x ^ (1u<<n)); }
__attribute__((noinline)) u64 tog8_wide_u(u8 x,u32 n){ return (u8)(x ^ (1u<<n)); }
__attribute__((noinline)) int tog16_neg(s16 x,u32 n){ s16 y=(s16)(x ^ (1u<<n)); return y < 0; }
__attribute__((noinline)) s64 tog16_wide_s(s16 x,u32 n){ return (s16)(x ^ (1u<<n)); }
__attribute__((noinline)) u64 tog16_wide_u(u16 x,u32 n){ return (u16)(x ^ (1u<<n)); }
__attribute__((noinline)) int tog32_neg(s32 x,u32 n){ u32 y=(u32)x ^ (1u<<n); return (s32)y < 0; }
__attribute__((noinline)) s64 tog32_wide_s(s32 x,u32 n){ return (s32)((u32)x ^ (1u<<n)); }
__attribute__((noinline)) u64 tog32_wide_u(u32 x,u32 n){ return x ^ (1u<<n); }
__attribute__((noinline)) int tog64_neg(s64 x,u64 n){ u64 y=(u64)x ^ (1ull<<n); return (s64)y < 0; }
__attribute__((noinline)) u64 tog64(u64 x,u64 n){ return x ^ (1ull<<n); }
u64 bit_toggle_checksum(void){
  h = 1469598103934665603ull;
  static const u32 pat[]={0u,0xffffffffu,0x80000000u,0x7fffffffu,0xaaaaaaaau,0x55555555u,0xdeadbeefu,1u};
  for(unsigned p=0;p<8;p++){
    u32 x=pat[p];
    for(u32 n=0;n<8;n++){
      mix(tog8_neg((s8)x,n));
      mix((u64)tog8_wide_s((s8)x,n)); mix(tog8_wide_u((u8)x,n));
      mix((u64)tog8_wide_s((s8)tog8_wide_u((u8)x,n),n));
    }
    for(u32 n=0;n<16;n++){
      mix(tog16_neg((s16)x,n));
      mix((u64)tog16_wide_s((s16)x,n)); mix(tog16_wide_u((u16)x,n));
    }
    for(u32 n=0;n<32;n++){
      mix(tog32_neg((s32)x,n));
      mix((u64)tog32_wide_s((s32)x,n)); mix(tog32_wide_u(x,n));
    }
    u64 xx=((u64)x<<32)|pat[(p+3)&7];
    for(u64 n=0;n<64;n++){
      mix(tog64_neg((s64)xx,n)); mix(tog64(xx,n));
      mix(tog64(tog64(xx,n),n));
    }
  }
  return h;
}
