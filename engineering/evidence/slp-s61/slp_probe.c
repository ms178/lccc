/* BB-SLP domain probe — straight-line superword shapes only.
 *
 * Every function here is a shape the SLP domain owns (no loop the loop
 * vectorizer can take).  Compiled at -O2 -march=x86-64-v3 and ranked
 * against gcc 16.2 / clang 23.1 / icx / icc by the codegen oracle.
 */
#include <stdint.h>
#include <stddef.h>

/* ---- 1. straight-line ALU packs, every lane width ------------------ */

void add_i32x4(const int32_t *restrict a, const int32_t *restrict b,
               int32_t *restrict d) {
  d[0] = a[0] + b[0]; d[1] = a[1] + b[1];
  d[2] = a[2] + b[2]; d[3] = a[3] + b[3];
}
void add_i32x8(const int32_t *restrict a, const int32_t *restrict b,
               int32_t *restrict d) {
  for (int i = 0; i < 8; i++) d[i] = a[i] + b[i];
}
void sub_i64x4(const int64_t *restrict a, const int64_t *restrict b,
               int64_t *restrict d) {
  d[0] = a[0] - b[0]; d[1] = a[1] - b[1];
  d[2] = a[2] - b[2]; d[3] = a[3] - b[3];
}
void mul_i16x8(const int16_t *restrict a, const int16_t *restrict b,
               int16_t *restrict d) {
  for (int i = 0; i < 8; i++) d[i] = (int16_t)(a[i] * b[i]);
}
void mul_i32x4(const int32_t *restrict a, const int32_t *restrict b,
               int32_t *restrict d) {
  d[0] = a[0] * b[0]; d[1] = a[1] * b[1];
  d[2] = a[2] * b[2]; d[3] = a[3] * b[3];
}
void xor_i64x4(const uint64_t *restrict a, const uint64_t *restrict b,
               uint64_t *restrict d) {
  d[0] = a[0] ^ b[0]; d[1] = a[1] ^ b[1];
  d[2] = a[2] ^ b[2]; d[3] = a[3] ^ b[3];
}
void andor_i32x4(const uint32_t *restrict a, const uint32_t *restrict b,
                 const uint32_t *restrict c, uint32_t *restrict d) {
  d[0] = (a[0] & b[0]) | c[0]; d[1] = (a[1] & b[1]) | c[1];
  d[2] = (a[2] & b[2]) | c[2]; d[3] = (a[3] & b[3]) | c[3];
}

/* ---- 2. FP lanes --------------------------------------------------- */

void fadd_f64x2(const double *restrict a, const double *restrict b,
                double *restrict d) {
  d[0] = a[0] + b[0]; d[1] = a[1] + b[1];
}
void fma_f64x2(const double *restrict a, const double *restrict b,
               const double *restrict c, double *restrict d) {
  d[0] = a[0] * b[0] + c[0]; d[1] = a[1] * b[1] + c[1];
}
void fma_f32x4(const float *restrict a, const float *restrict b,
               const float *restrict c, float *restrict d) {
  d[0] = a[0] * b[0] + c[0]; d[1] = a[1] * b[1] + c[1];
  d[2] = a[2] * b[2] + c[2]; d[3] = a[3] * b[3] + c[3];
}
void fadd_f32x4(const float *restrict a, const float *restrict b,
                float *restrict d) {
  d[0] = a[0] + b[0]; d[1] = a[1] + b[1];
  d[2] = a[2] + b[2]; d[3] = a[3] + b[3];
}

/* ---- 3. struct-field streams (the v8 unlock) ----------------------- */

struct P4 { double x, y, z, w; };
void struct_copy_q4(const struct P4 *q, struct P4 *p) { *p = *q; }
struct P3 { double x, y, z; };
void struct_copy_q3(const struct P3 *restrict q, struct P3 *restrict p) {
  p->x = q->x; p->y = q->y; p->z = q->z;
}
struct Pm { float m[16]; };
void mat4_col_copy(const struct Pm *restrict q, struct Pm *restrict p) {
  for (int i = 0; i < 4; i++) p->m[i] = q->m[i];
  for (int i = 0; i < 4; i++) p->m[i + 4] = q->m[i + 4];
}

/* ---- 4. splat / zero runs ------------------------------------------ */

void zero_i32x4(int32_t *d) { d[0] = d[1] = d[2] = d[3] = 0; }
void splat_i32x4(int32_t *d, int32_t v) { d[0] = d[1] = d[2] = d[3] = v; }
void zero_i64x4(int64_t *d) { d[0] = d[1] = d[2] = d[3] = 0; }
void zero_i8x16(uint8_t *d) { for (int i = 0; i < 16; i++) d[i] = 0; }

/* ---- 5. shifts / rotates (v5) -------------------------------------- */

void shl_i32x4(const uint32_t *restrict a, uint32_t *restrict d) {
  d[0] = a[0] << 3; d[1] = a[1] << 3; d[2] = a[2] << 3; d[3] = a[3] << 3;
}
void lshr_i64x4(const uint64_t *restrict a, uint64_t *restrict d) {
  d[0] = a[0] >> 5; d[1] = a[1] >> 5; d[2] = a[2] >> 5; d[3] = a[3] >> 5;
}
void rotl_i32x4(const uint32_t *restrict a, uint32_t *restrict d) {
  for (int i = 0; i < 4; i++) d[i] = (a[i] << 7) | (a[i] >> 25);
}

/* ---- 6. min / max / select (v7) ------------------------------------ */

void fmax_f32x4(const float *restrict a, const float *restrict b,
                float *restrict d) {
  for (int i = 0; i < 4; i++) d[i] = a[i] > b[i] ? a[i] : b[i];
}
void imax_i32x4(const int32_t *restrict a, const int32_t *restrict b,
                int32_t *restrict d) {
  for (int i = 0; i < 4; i++) d[i] = a[i] > b[i] ? a[i] : b[i];
}
void clamp_u8x16(const uint8_t *restrict a, uint8_t *restrict d) {
  for (int i = 0; i < 16; i++) d[i] = a[i] > 200 ? 200 : (a[i] < 16 ? 16 : a[i]);
}

/* ---- 7. SHA-256 message schedule (v8 follow-up #2) ------------------ */

static inline uint32_t ror32(uint32_t x, int n) {
  return (x >> n) | (x << (32 - n));
}
void sha256_schedule(const uint32_t *restrict w_in, uint32_t *restrict w) {
  for (int i = 0; i < 16; i++) w[i] = __builtin_bswap32(w_in[i]);
  for (int i = 16; i < 64; i += 2) {
    uint32_t a0 = w[i - 15], a1 = w[i - 14];
    uint32_t s0_0 = ror32(a0, 7) ^ ror32(a0, 18) ^ (a0 >> 3);
    uint32_t s0_1 = ror32(a1, 7) ^ ror32(a1, 18) ^ (a1 >> 3);
    uint32_t b0 = w[i - 2], b1 = w[i - 1];
    uint32_t s1_0 = ror32(b0, 17) ^ ror32(b0, 19) ^ (b0 >> 10);
    uint32_t s1_1 = ror32(b1, 17) ^ ror32(b1, 19) ^ (b1 >> 10);
    w[i] = s1_0 + w[i - 7] + s0_0 + w[i - 16];
    w[i + 1] = s1_1 + w[i - 6] + s0_1 + w[i - 15];
  }
}

/* ---- 8. pure reductions over straight-line lanes ------------------- */

int32_t hsum_i32x4(const int32_t *restrict a) {
  return a[0] + a[1] + a[2] + a[3];
}
double hsum_f64x4(const double *restrict a) {
  return a[0] + a[1] + a[2] + a[3];
}
float dot_f32x4(const float *restrict a, const float *restrict b) {
  return a[0]*b[0] + a[1]*b[1] + a[2]*b[2] + a[3]*b[3];
}
uint32_t popcnt_mix4(const uint32_t *restrict a) {
  return (uint32_t)(__builtin_popcount(a[0]) + __builtin_popcount(a[1])
                  + __builtin_popcount(a[2]) + __builtin_popcount(a[3]));
}

/* ---- 9. call-argument / return seeds (non-goal in v1) --------------- */

struct V4 { float x, y, z, w; };
struct V4 scale4(struct V4 v, float s) {
  struct V4 r; r.x = v.x * s; r.y = v.y * s; r.z = v.z * s; r.w = v.w * s;
  return r;
}
struct V4 add4(struct V4 a, struct V4 b) {
  struct V4 r; r.x = a.x+b.x; r.y = a.y+b.y; r.z = a.z+b.z; r.w = a.w+b.w;
  return r;
}
void sink(struct V4);
void call_arg_pack(const float *restrict a, float s) {
  struct V4 v; v.x = a[0]*s; v.y = a[1]*s; v.z = a[2]*s; v.w = a[3]*s;
  sink(v);
}

/* ---- 10. strided / permuted lanes (shuffle territory) --------------- */

void deinterleave(const float *restrict s, float *restrict x, float *restrict y) {
  for (int i = 0; i < 4; i++) { x[i] = s[2*i]; y[i] = s[2*i+1]; }
}
void interleave(const float *restrict x, const float *restrict y,
                float *restrict d) {
  for (int i = 0; i < 4; i++) { d[2*i] = x[i]; d[2*i+1] = y[i]; }
}
void rev_copy(const float *restrict a, float *restrict d) {
  for (int i = 0; i < 4; i++) d[i] = a[3-i];
}

/* ---- 11. widening / narrowing packs -------------------------------- */

void widen_i16_to_i32(const int16_t *restrict a, int32_t *restrict d) {
  for (int i = 0; i < 4; i++) d[i] = a[i];
}
void narrow_i32_to_i16(const int32_t *restrict a, int16_t *restrict d) {
  for (int i = 0; i < 8; i++) d[i] = (int16_t)a[i];
}
void u8_to_f32(const uint8_t *restrict a, float *restrict d) {
  for (int i = 0; i < 4; i++) d[i] = (float)a[i];
}

/* ---- 12. byte/half streams ----------------------------------------- */

void add_i8x16(const int8_t *restrict a, const int8_t *restrict b,
               int8_t *restrict d) {
  for (int i = 0; i < 16; i++) d[i] = (int8_t)(a[i] + b[i]);
}
void sub_i16x8(const int16_t *restrict a, const int16_t *restrict b,
               int16_t *restrict d) {
  for (int i = 0; i < 8; i++) d[i] = (int16_t)(a[i] - b[i]);
}
void abs_i16x8(const int16_t *restrict a, int16_t *restrict d) {
  for (int i = 0; i < 8; i++) { int16_t t = a[i]; d[i] = t < 0 ? (int16_t)-t : t; }
}

/* ---- 13. mixed / diamond pack graphs ------------------------------- */

void diamond(const uint32_t *restrict a, const uint32_t *restrict b,
             uint32_t *restrict d) {
  for (int i = 0; i < 4; i++) {
    uint32_t t = a[i] ^ b[i];
    d[i] = t + a[i] * 3u;
  }
}
void two_streams(const uint32_t *restrict a, const uint32_t *restrict b,
                 uint32_t *restrict d, uint32_t *restrict e) {
  for (int i = 0; i < 4; i++) { d[i] = a[i] + b[i]; e[i] = a[i] - b[i]; }
}

/* ---- 14. byte-copy / memcmp shapes (glibc) ------------------------- */

int cmp_q4(const uint64_t *restrict a, const uint64_t *restrict b) {
  uint64_t x0 = a[0] ^ b[0], x1 = a[1] ^ b[1], x2 = a[2] ^ b[2],
           x3 = a[3] ^ b[3];
  uint64_t all = x0 | x1 | x2 | x3;
  return all != 0;
}
void copy_bytes(uint8_t *restrict d, const uint8_t *restrict s) {
  for (int i = 0; i < 32; i++) d[i] = s[i];
}

/* ---- 15. gather / broadcast leaves --------------------------------- */

void gather4(const uint32_t *restrict a, size_t i, size_t j, size_t k,
             size_t l, uint32_t *restrict d) {
  d[0] = a[i] + 1; d[1] = a[j] + 1; d[2] = a[k] + 1; d[3] = a[l] + 1;
}
void bc_mul(const float *restrict a, float s, float *restrict d) {
  for (int i = 0; i < 4; i++) d[i] = a[i] * s;
}
