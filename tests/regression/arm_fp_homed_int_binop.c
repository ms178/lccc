// Regression: integer BinOp destination with an FP register home (d8-d14 /
// d16-d31) must not reach callee_saved_name ("invalid ARM register index"
// ICE). The register-direct ALU path must filter FP-homed dests and
// operand_to_callee_reg must stage FP-homed sources through x0.
// Derived from aarch64_fuzz seed 17 scenario at -O2.
typedef unsigned int u32; typedef int i32; typedef unsigned long long u64;
static volatile u32 V = 2241903810u;
static u32 H = 2166136261u;
static void mix(u32 v) { H = (H ^ v) * 16777619u; }
static u32 f2u(float f) { union { float f; u32 u; } c; c.f = f; return c.u; }
static u32 d2lo(double d) { union { double d; u64 u; } c; c.d = d; return (u32)c.u; }
static u32 d2hi(double d) { union { double d; u64 u; } c; c.d = d; return (u32)(c.u >> 32); }
static double m[16][16]; static int im[64];
u32 probe(i32 seed, i32 it) {
  double a = 1.0, b = 2.0, c = 3.0, d = 4.0, e = 5.0, f = 6.0, g = 7.0, h = 8.0;
  for (int i = 0; i < 64; i++) im[i] = i ^ seed;
  for (int i = 0; i < 16; i++)
    for (int j = 0; j < 16; j++)
      m[i][j] = (double)((im[i & 63] + j) & 31);
  for (int k = 0; k < 40; k++) {
    a += m[k & 15][k & 15] * 0.25; b -= a * 0.5; c += b * 0.125;
    d += c * 0.0625; e += d * 2.0; f -= e; g += f; h = h * 1.01 + g * 0.001;
  }
  mix(f2u(a)); mix(d2hi(h));
  return H;
}
int printf(const char *, ...);
int main(void) {
    u32 r = 0;
    for (int it = 0; it < 4; it++) r ^= probe((i32)(V + it), it);
    printf("%08x\n", r);
    return 0;
}
