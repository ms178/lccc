/* Isolated unsigned tail of Firefox SpecularLightingSoftware::LightPixel.
 * LLVM #46312 and PR #227303. No bounds or scale-range assumptions.
 * Keep external calls uninlined so the harness cannot specialize inputs. */
typedef unsigned int u32;
typedef unsigned char u8;
static u32 channel(u8 x, u32 scale) {
    u32 v = (scale * x) >> 15;
    return v < 255U ? v : 255U;
}
static u32 umax(u32 a, u32 b) { return a > b ? a : b; }
void lightpixel(u8 *p, u32 scale) {
    p[0] = channel(p[0], scale);
    p[1] = channel(p[1], scale);
    p[2] = channel(p[2], scale);
    p[3] = umax(p[0], umax(p[1], p[2]));
}
void lightpixel_separate(const u8 *in, u8 *out, u32 scale) {
    u32 b = channel(in[0], scale), g = channel(in[1], scale);
    u32 r = channel(in[2], scale);
    out[0] = b; out[1] = g; out[2] = r;
    out[3] = b > g ? b : g;
    out[3] = (u32)out[3] > r ? out[3] : r;
}
u32 lightpixel_value(u32 color, u32 scale) {
    u32 b = channel((u8)color, scale);
    u32 g = channel((u8)(color >> 8), scale);
    u32 r = channel((u8)(color >> 16), scale);
    u32 a = b > g ? b : g;
    a = a > r ? a : r;
    return b | (g << 8) | (r << 16) | (a << 24);
}
/* Consecutive widened outputs expose the arithmetic independently of
 * non-power-of-two packing, narrowing and the external max reduction. */
void lightpixel_wide4(const u32 *restrict in, u32 *restrict out, u32 scale) {
#define LANE(i) do { u32 v = (scale * in[i]) >> 15; out[i] = v < 255U ? v : 255U; } while (0)
    LANE(0); LANE(1); LANE(2); LANE(3);
#undef LANE
}
void umin4(const u32 *restrict a, const u32 *restrict b, u32 *restrict out) {
#define LANE(i) out[i] = a[i] < b[i] ? a[i] : b[i]
    LANE(0); LANE(1); LANE(2); LANE(3);
#undef LANE
}
void umax4(const u32 *restrict a, const u32 *restrict b, u32 *restrict out) {
#define LANE(i) out[i] = a[i] > b[i] ? a[i] : b[i]
    LANE(0); LANE(1); LANE(2); LANE(3);
#undef LANE
}
/* Negative legality controls: only four scattered bytes may be written;
 * volatile accesses must not become packed accesses. */
void lightpixel_scattered(const u8 *in, u8 *out, u32 scale) {
    u32 b = channel(in[0], scale), g = channel(in[1], scale);
    u32 r = channel(in[2], scale), a = b > g ? b : g;
    out[0] = b; out[2] = g; out[4] = r;
    out[6] = a > r ? a : r;
}
void lightpixel_volatile(volatile u8 *p, u32 scale) {
    p[0] = channel(p[0], scale);
    p[1] = channel(p[1], scale);
    p[2] = channel(p[2], scale);
    u32 b = p[0], g = p[1], r = p[2];
    u32 a = b > g ? b : g;
    p[3] = a > r ? a : r;
}
void lightpixel_interference(u8 *p, u8 *q, u32 scale) {
    p[0] = channel(p[0], scale);
    *q = 173; /* q is allowed to alias any channel. */
    p[1] = channel(p[1], scale);
    p[2] = channel(p[2], scale);
    u32 a = p[0] > p[1] ? p[0] : p[1];
    p[3] = a > (u32)p[2] ? a : p[2];
}

void lightpixel_wide8(const u32 *restrict in, u32 *restrict out, u32 scale) {
#define LANE(i) do { u32 v = (scale * in[i]) >> 15; out[i] = v < 255U ? v : 255U; } while (0)
    LANE(0); LANE(1); LANE(2); LANE(3); LANE(4); LANE(5); LANE(6); LANE(7);
#undef LANE
}

void umin8(const u32 *restrict a, const u32 *restrict b, u32 *restrict out) {
#define LANE(i) out[i] = a[i] < b[i] ? a[i] : b[i]
    LANE(0); LANE(1); LANE(2); LANE(3); LANE(4); LANE(5); LANE(6); LANE(7);
#undef LANE
}

void umax8(const u32 *restrict a, const u32 *restrict b, u32 *restrict out) {
#define LANE(i) out[i] = a[i] > b[i] ? a[i] : b[i]
    LANE(0); LANE(1); LANE(2); LANE(3); LANE(4); LANE(5); LANE(6); LANE(7);
#undef LANE
}
