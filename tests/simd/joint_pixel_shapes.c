typedef unsigned char u8;
typedef unsigned int u32;
static u32 min(u32 a, u32 b) { return a < b ? a : b; }
static u32 max(u32 a, u32 b) { return a > b ? a : b; }
static u32 channel(u8 c, u32 s) { return min(((u32)c * s) >> 15, 255U); }
#define PIXEL(N, A, B, G, R) \
void joint_alpha##N(const u8 *in, u8 *out, u32 s) { \
    u32 b = channel(in[0], s), g = channel(in[1], s), r = channel(in[2], s); \
    out[B] = b; out[G] = g; out[R] = r; out[A] = max(b, max(g, r)); \
}
PIXEL(0,0,1,2,3)
PIXEL(1,1,0,2,3)
PIXEL(2,2,0,1,3)
PIXEL(3,3,0,1,2)
void joint_four(const u8 *in, u8 *out, u32 s) {
    u32 b = channel(in[0],s), g = channel(in[1],s);
    u32 r = channel(in[2],s), a = channel(in[3],s);
    out[0] = b; out[1] = g; out[2] = r; out[3] = max(a,max(b,max(g,r)));
}
void joint_retired_read(const u8 *in, u8 *out, u32 s) {
    out[0] = in[0];
    u32 b = channel(out[0], s), g = channel(in[1], s), r = channel(in[2], s);
    out[0] = b; out[1] = g; out[2] = r; out[3] = max(b,max(g,r));
}
void joint_intervening_write(const u8 *in, u8 *out, u32 s) {
    u32 b = channel(in[0], s);
    out[1] = in[2];
    u32 g = channel(in[1], s), r = channel(in[2], s);
    out[0] = b; out[1] = g; out[2] = r; out[3] = max(b,max(g,r));
}
void joint_volatile(const u8 *in, u8 *out, u32 s) {
    const volatile u8 *v = in;
    u32 b = channel(v[0],s), g = channel(v[1],s), r = channel(v[2],s);
    out[0] = b; out[1] = g; out[2] = r; out[3] = max(b,max(g,r));
}
#define FOUR(N, A, B, G, R) \
void joint_four_alpha##N(const u8 *in, u8 *out, u32 s) { \
    u32 b = channel(in[0],s), g = channel(in[1],s); \
    u32 r = channel(in[2],s), a = channel(in[3],s); \
    out[B] = b; out[G] = g; out[R] = r; out[A] = max(a,max(b,max(g,r))); \
}
FOUR(0,0,1,2,3)
FOUR(1,1,0,2,3)
FOUR(2,2,0,1,3)
u32 joint_value_reverse(u32 p, u32 s) {
    u32 b=channel((u8)(p>>16),s), g=channel((u8)(p>>8),s), r=channel((u8)p,s);
    return b | (g<<8) | (r<<16) | (max(b,max(g,r))<<24);
}
u32 joint_value_four(u32 p, u32 s) {
    u32 b=channel((u8)p,s), g=channel((u8)(p>>8),s);
    u32 r=channel((u8)(p>>16),s), a=channel((u8)(p>>24),s);
    return b | (g<<8) | (r<<16) | (max(a,max(b,max(g,r)))<<24);
}
