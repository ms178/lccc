/* Runtime benchmark for the nested-diamond clamp / abs / saturating family.
 *
 * Data-dependent inputs (a cheap LCG, deliberately NOT sorted) so the
 * branches are hard to predict — that is the case where if-conversion pays
 * and the case a sorted microbenchmark would hide.
 */
#include <stdint.h>
#include <stdio.h>
#include <time.h>
#include <string.h>

#define N 4096
#define REPS 4000

static uint8_t  src8[N], dst8[N];
static int16_t  src16[N], dst16[N];
static int32_t  src32[N], dst32[N];
static uint64_t sink;

void clamp_u8(const uint8_t *a, uint8_t *d) {
  for (int i = 0; i < N; i++) d[i] = a[i] > 200 ? 200 : (a[i] < 16 ? 16 : a[i]);
}
void clamp_i32(const int32_t *a, int32_t *d) {
  for (int i = 0; i < N; i++) d[i] = a[i] > 200 ? 200 : (a[i] < 16 ? 16 : a[i]);
}
void abs_i16(const int16_t *a, int16_t *d) {
  for (int i = 0; i < N; i++) { int16_t t = a[i]; d[i] = t < 0 ? (int16_t)-t : t; }
}
void sat_add_i16(const int16_t *a, const int16_t *b, int16_t *d) {
  for (int i = 0; i < N; i++) {
    int s = (int)a[i] + (int)b[i];
    d[i] = s > 32767 ? 32767 : (s < -32768 ? -32768 : (int16_t)s);
  }
}

/* Opaque zero: a noinline call the compiler cannot fold, so the REPS loop
 * cannot be collapsed into a single call (both lccc and GCC otherwise prove
 * the repeated identical call idempotent and delete 3999 of 4000 iterations
 * — measured: gcc -O3 clamp_u8 reported 0.0000 s).  The value comes from a
 * VOLATILE so the call is not `const`: a pure noinline zero would be CSE'd
 * to one call and hoisted out of the loop, collapsing it just the same
 * (also measured: 0.0004 s for 16.4 M lane clamps, i.e. 16 lanes/cycle —
 * physically impossible, which is how the defect was caught). */
static volatile int g_zero = 0;
__attribute__((noinline)) static int opaque_zero(void) { return g_zero; }

static double now(void) {
  struct timespec ts; clock_gettime(CLOCK_MONOTONIC, &ts);
  return ts.tv_sec + ts.tv_nsec * 1e-9;
}

int main(void) {
  uint64_t s = 0x243F6A8885A308D3ull;
  for (int i = 0; i < N; i++) {
    s ^= s << 13; s ^= s >> 7; s ^= s << 17;
    src8[i]  = (uint8_t)(s & 0xFF);
    src16[i] = (int16_t)((int32_t)(s >> 16) % 60000 - 30000);
    src32[i] = (int32_t)(s >> 8) % 1000 - 500;
  }
  double t0, t1, best[4];
  for (int k = 0; k < 4; k++) best[k] = 1e30;

  for (int r = 0; r < 7; r++) {
    t0 = now();
    for (int i = 0; i < REPS; i++) clamp_u8(src8 + opaque_zero(), dst8 + opaque_zero());
    t1 = now(); if (t1-t0 < best[0]) best[0] = t1-t0;
    sink += dst8[0] + dst8[N-1];

    t0 = now();
    for (int i = 0; i < REPS; i++) clamp_i32(src32 + opaque_zero(), dst32 + opaque_zero());
    t1 = now(); if (t1-t0 < best[1]) best[1] = t1-t0;
    sink += dst32[0] + dst32[N-1];

    t0 = now();
    for (int i = 0; i < REPS; i++) abs_i16(src16 + opaque_zero(), dst16 + opaque_zero());
    t1 = now(); if (t1-t0 < best[2]) best[2] = t1-t0;
    sink += dst16[0] + dst16[N-1];

    t0 = now();
    for (int i = 0; i < REPS; i++) sat_add_i16(src16 + opaque_zero(), src16, dst16 + opaque_zero());
    t1 = now(); if (t1-t0 < best[3]) best[3] = t1-t0;
    sink += dst16[0] + dst16[N-1];
  }
  /* Print a checksum first so the harness can verify correctness, then times. */
  uint64_t cs = 0;
  for (int i = 0; i < N; i++) cs = cs * 31 + (uint8_t)dst8[i];
  clamp_u8(src8, dst8);
  for (int i = 0; i < N; i++) cs = cs * 31 + (uint8_t)dst8[i];
  clamp_i32(src32, dst32);
  for (int i = 0; i < N; i++) cs = cs * 31 + (uint32_t)dst32[i];
  abs_i16(src16, dst16);
  for (int i = 0; i < N; i++) cs = cs * 31 + (uint16_t)dst16[i];
  sat_add_i16(src16, src16, dst16);
  for (int i = 0; i < N; i++) cs = cs * 31 + (uint16_t)dst16[i];
  printf("checksum %llu\n", (unsigned long long)cs);
  printf("clamp_u8 %.4f\nclamp_i32 %.4f\nabs_i16 %.4f\nsat_add_i16 %.4f\n",
         best[0], best[1], best[2], best[3]);
  if (sink == 12345) printf("unreachable\n");
  return 0;
}
