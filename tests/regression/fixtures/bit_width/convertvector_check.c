/* Freestanding checksum over the __builtin_convertvector kernels
 * (convertvector.c).
 *
 * PIN is the value GCC 16.2 and Clang 23.1.4 both compute at -O0, -O1,
 * -O2, -O3 and -Os (hosted x86-64 build of both files), that a
 * -fsanitize=undefined trap build of both compilers reproduces without
 * trapping, and that GCC 16.2 -m32 and Clang 23.1.4 for AArch64 and
 * RISC-V reproduce under QEMU.  Every input stays inside its conversion's
 * defined range (no float -> integer overflow, no negative float ->
 * unsigned, no double -> float beyond FLT_MAX, no NaN, whose payload
 * RISC-V canonicalizes), so the value is target-independent.  The rows
 * pin the rounding cases: 2^24 + 1 and 2^53 + 1 to float/double, the
 * double -> float ties, unsigned values with the top bit set, and
 * truncation toward zero of negative fractions. */
typedef unsigned long long u64;

#define PIN 16501472973454287012ull

typedef union {
  unsigned char b[32];
  int i[8];
  unsigned u[8];
  float f[8];
  double d[4];
  long long l[4];
  unsigned long long ul[4];
  short s[16];
  signed char c[32];
} __attribute__((aligned(32))) row;

typedef void kernel(const void *, void *);
kernel cv_i32_f32, cv_f32_i32, cv_u32_f32, cv_f32_u32, cv_f64_i64, cv_i64_f64,
    cv_u64_f64, cv_f64_u64, cv_s16_u16, cv_s8_u8, cv_f64_f32_2, cv_f32_f64_2,
    cv_i32_i16, cv_i16_i32, cv_u8_i32, cv_s8_f32, cv_f32_f64, cv_f64_f32,
    cv_i32_f64, cv_f64_i32, cv_u32_i64, cv_i64_i32, cv_i32_i32, cv_i32_u32,
    cv_i32_f32_2, cv_f32_i32_2;

#define I32_MIN (-2147483647 - 1)
#define I64_MIN (-9223372036854775807ll - 1)

static const row ints[] = {
    {.i = {-7, 16777217, I32_MIN, 2147483647, 0, 1, -1, 123456789}},
    {.i = {2147483520, -16777215, 8388609, 65536, -2, 33554435, -33554435, 7}},
};
static const row uints[] = {
    {.u = {0x80000000u, 0xffffffffu, 1u, 0xffffff7fu, 0u, 16777217u, 0xfffffffeu, 3000000000u}},
    {.u = {0x7fffffffu, 0x80000081u, 0xffffff80u, 4294967040u, 2u, 3u, 5u, 8u}},
};
static const row floats_s[] = { /* in range for int */
    {.f = {-1.9f, 2147483520.0f, 100.5f, -0.0f, 0.5f, -0.5f, -2147483648.0f, 1e-30f}},
    {.f = {16777216.0f, -16777216.0f, 3.5f, -3.5f, 1.0e9f, -1.0e9f, 2.5f, -2.5f}},
};
static const row floats_u[] = { /* in range for unsigned */
    {.f = {3e9f, 4294967040.0f, 0.75f, 16777216.0f, 0.0f, 1.5f, 2147483648.0f, 65535.9f}},
    {.f = {2147483520.0f, 4294967040.0f, 0.999f, 1e-40f, 3.0f, 7.75f, 1048576.5f, 12.0f}},
};
static const row floats_any[] = { /* finite, infinite and subnormal */
    {.f = {1.1f, -0.0f, 1e-40f, 3.4028234663852886e38f, -1e30f, 0.1f, 65504.0f, -2.0f}},
    {.f = {__builtin_inff(), -__builtin_inff(), 1.17549435e-38f, 7.0f, -7.25f, 0.3f, 1e10f, -1e-10f}},
};
static const row doubles_i32[] = { /* in range for int */
    {.d = {-3.99, 123456789.75, -2147483648.0, 0.5}},
    {.d = {2147483647.0, -0.999, 1e-300, 65536.5}},
};
static const row doubles_i64[] = { /* in range for long long */
    {.d = {-3.99, 1e15, 9223372036854774784.0, -9223372036854775808.0}},
    {.d = {4503599627370497.5, -0.5, 1e-300, -1e18}},
};
static const row doubles_u64[] = { /* in range for unsigned long long */
    {.d = {18446744073709549568.0, 0.5, 9223372036854775808.0, 4503599627370497.0}},
    {.d = {9223372036854774784.0, 1e19, 0.0, 3.75}},
};
static const row doubles_f32[] = { /* within float's range */
    {.d = {-3.99, 1e15, 1e-50, 3.4028234663852886e38}},
    {.d = {1.0000000596046448, 1.0000001788139343, -1e-40, 65504.0}},
    {.d = {__builtin_inf(), -0.0, 0.1, 2.0000002384185791}},
};
static const row longs[] = {
    {.l = {(1ll << 53) + 1, -5, I64_MIN, 9223372036854775807ll}},
    {.l = {0x123456789abcdef0ll, -1, 0x7fffffff80000000ll, 0x00000000ffffffffll}},
};
static const row ulongs[] = {
    {.ul = {0xffffffffffffffffull, 1ull << 63, (1ull << 53) + 1, 12345ull}},
    {.ul = {0x8000000000000401ull, 0x7fffffffffffffffull, 0ull, 0x10000000000000ull}},
};
static const row shorts[] = {
    {.s = {-1, -32768, 32767, 0, 1, -2, 300, -300, 5, -5, 255, -256, 4096, -4096, 77, -77}},
};
static const row chars[] = {
    {.c = {-128, -1, 0, 1, 127, 64, -64, 3, -3, 100, -100, 9, -9, 50, -50, 7,
           12, -12, 34, -34, 56, -56, 78, -78, 90, -90, 11, -11, 22, -22, 33, -33}},
};
static const row uchars[] = {
    {.b = {200, 1, 255, 0, 128, 127, 9, 90, 13, 250, 31, 64, 99, 100, 101, 7}},
};

static u64 h = 1469598103934665603ull;

static void mix(const unsigned char *p, unsigned n) {
  for (unsigned k = 0; k < n; k++) {
    h ^= p[k];
    h *= 1099511628211ull;
  }
}

static void run(kernel *k, const row *rows, unsigned nrows, unsigned out_bytes) {
  for (unsigned r = 0; r < nrows; r++) {
    row out;
    for (unsigned k2 = 0; k2 < 32; k2++)
      out.b[k2] = 0xa5;
    k(&rows[r], &out);
    mix(out.b, out_bytes);
  }
}

#define RUN(k, rows, out_bytes) run(k, rows, sizeof(rows) / sizeof(rows[0]), out_bytes)

u64 convertvector_checksum(void) {
  h = 1469598103934665603ull;
  RUN(cv_i32_f32, ints, 16);
  RUN(cv_f32_i32, floats_s, 16);
  RUN(cv_u32_f32, uints, 16);
  RUN(cv_f32_u32, floats_u, 16);
  RUN(cv_f64_i64, doubles_i64, 16);
  RUN(cv_i64_f64, longs, 16);
  RUN(cv_u64_f64, ulongs, 16);
  RUN(cv_f64_u64, doubles_u64, 16);
  RUN(cv_s16_u16, shorts, 16);
  RUN(cv_s8_u8, chars, 16);
  RUN(cv_f64_f32_2, doubles_f32, 8);
  RUN(cv_f32_f64_2, floats_any, 16);
  RUN(cv_i32_i16, ints, 8);
  RUN(cv_i16_i32, shorts, 16);
  RUN(cv_u8_i32, uchars, 32);
  RUN(cv_s8_f32, chars, 32);
  RUN(cv_f32_f64, floats_any, 32);
  RUN(cv_f64_f32, doubles_f32, 16);
  RUN(cv_i32_f64, ints, 32);
  RUN(cv_f64_i32, doubles_i32, 16);
  RUN(cv_u32_i64, uints, 32);
  RUN(cv_i64_i32, longs, 16);
  RUN(cv_i32_i32, ints, 16);
  RUN(cv_i32_u32, ints, 16);
  RUN(cv_i32_f32_2, ints, 8);
  RUN(cv_f32_i32_2, floats_s, 8);
  return h;
}

int test(void) { return convertvector_checksum() != PIN; }
