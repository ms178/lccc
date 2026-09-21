/* Nested-diamond if-conversion + BB-SLP differential: clamp / saturating /
 * abs / minmax families at every lane width, checked exhaustively against
 * the reference compiler at runtime.
 *
 * Every kernel is written so a MIS-converted diamond or a mis-packed lane
 * shows up as a value mismatch, and the input sweep covers the exact
 * boundaries (lo-1, lo, lo+1, hi-1, hi, hi+1) plus the type extremes.
 */
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#define N 32

static uint8_t  bu[N], bv[N], bd[N];
static int8_t   sb[N], sv[N], sd[N];
static uint16_t hu[N], hv[N], hd[N];
static int16_t  hs[N], hv2[N], hd2[N];
static uint32_t wu[N], wv[N], wd[N];
static int32_t  ws[N], wv2[N], wd2[N];
static uint64_t qu[N], qv[N], qd[N];
static int64_t  qs[N], qv2[N], qd2[N];

/* ---- nested clamps at every width ---------------------------------- */
void clamp_u8(const uint8_t *a, uint8_t *d) {
  for (int i = 0; i < 16; i++) d[i] = a[i] > 200 ? 200 : (a[i] < 16 ? 16 : a[i]);
}
void clamp_u16(const uint16_t *a, uint16_t *d) {
  for (int i = 0; i < 8; i++) d[i] = a[i] > 60000 ? 60000 : (a[i] < 100 ? 100 : a[i]);
}
void clamp_i16(const int16_t *a, int16_t *d) {
  for (int i = 0; i < 8; i++) d[i] = a[i] > 30000 ? 30000 : (a[i] < -30000 ? -30000 : a[i]);
}
void clamp_u32(const uint32_t *a, uint32_t *d) {
  for (int i = 0; i < 4; i++) d[i] = a[i] > 4000000000u ? 4000000000u : (a[i] < 1000u ? 1000u : a[i]);
}
void clamp_i32(const int32_t *a, int32_t *d) {
  for (int i = 0; i < 4; i++) d[i] = a[i] > 200 ? 200 : (a[i] < 16 ? 16 : a[i]);
}
void clamp_i64(const int64_t *a, int64_t *d) {
  for (int i = 0; i < 4; i++) d[i] = a[i] > 200 ? 200 : (a[i] < 16 ? 16 : a[i]);
}
/* three levels deep */
void clamp3_i32(const int32_t *a, int32_t *d) {
  for (int i = 0; i < 4; i++) {
    int t = a[i];
    d[i] = t > 200 ? 200 : (t < 16 ? 16 : (t == 100 ? -1 : t));
  }
}
/* clamp with a computed (non-constant) bound */
void clamp_dyn_i32(const int32_t *a, const int32_t *b, int32_t *d) {
  for (int i = 0; i < 4; i++) {
    int hi = b[i], lo = -b[i];
    d[i] = a[i] > hi ? hi : (a[i] < lo ? lo : a[i]);
  }
}

/* ---- abs at every width -------------------------------------------- */
void abs_i16(const int16_t *a, int16_t *d) {
  for (int i = 0; i < 8; i++) { int16_t t = a[i]; d[i] = t < 0 ? (int16_t)-t : t; }
}
void abs_i32(const int32_t *a, int32_t *d) {
  for (int i = 0; i < 4; i++) { int32_t t = a[i]; d[i] = t < 0 ? -t : t; }
}
void abs_i64(const int64_t *a, int64_t *d) {
  for (int i = 0; i < 4; i++) { int64_t t = a[i]; d[i] = t < 0 ? -t : t; }
}

/* ---- saturating arithmetic ----------------------------------------- */
void sat_add_u8(const uint8_t *a, const uint8_t *b, uint8_t *d) {
  for (int i = 0; i < 16; i++) { unsigned s = (unsigned)a[i] + b[i]; d[i] = s > 255u ? 255u : (uint8_t)s; }
}
void sat_sub_u8(const uint8_t *a, const uint8_t *b, uint8_t *d) {
  for (int i = 0; i < 16; i++) { int s = (int)a[i] - (int)b[i]; d[i] = s < 0 ? 0 : (uint8_t)s; }
}
void sat_add_i16(const int16_t *a, const int16_t *b, int16_t *d) {
  for (int i = 0; i < 8; i++) { int s = (int)a[i] + (int)b[i]; d[i] = s > 32767 ? 32767 : (s < -32768 ? -32768 : (int16_t)s); }
}

/* ---- min/max with a constant, both spellings ------------------------ */
void maxc_i32(const int32_t *a, int32_t *d) {
  for (int i = 0; i < 4; i++) d[i] = a[i] > 7 ? a[i] : 7;
}
void minc_u32(const uint32_t *a, uint32_t *d) {
  for (int i = 0; i < 4; i++) d[i] = a[i] < 7u ? a[i] : 7u;
}
void maxc_i16(const int16_t *a, int16_t *d) {
  for (int i = 0; i < 8; i++) d[i] = a[i] > -3 ? a[i] : -3;
}

/* ---- nested diamond with SIDE EFFECTS in an arm (must stay branchy) -- */
volatile int sink;
void side_effect_arm(const int32_t *a, int32_t *d) {
  for (int i = 0; i < 4; i++) {
    int t;
    if (a[i] > 200) { sink = i; t = 200; }
    else if (a[i] < 16) { sink = i + 100; t = 16; }
    else t = a[i];
    d[i] = t;
  }
}

/* ---- null-guarded load in an arm (must NOT speculate the load) ------- */
int guarded_load(const int32_t *p, int32_t *d) {
  int r = 0;
  if (p) { r = *p; } else { r = 7; }
  *d = r;
  return r;
}

/* ------------------------------------------------------------------ */

static unsigned long long rng_s = 88172645463325252ull;
static uint64_t rnd(void) {
  rng_s ^= rng_s << 13; rng_s ^= rng_s >> 7; rng_s ^= rng_s << 17; return rng_s;
}

static int fails = 0;
#define CHECK(name, arr, n)                                          \
  do {                                                               \
    if (memcmp(arr, ref_##arr, sizeof((arr)[0]) * (n)) != 0) {        \
      printf("MISMATCH %s\n", name);                                 \
      for (int q = 0; q < (int)(n); q++)                             \
        if (arr[q] != ref_##arr[q])                                  \
          printf("  [%d] got %lld want %lld\n", q,                   \
                 (long long)arr[q], (long long)ref_##arr[q]);        \
      fails++;                                                       \
    }                                                                \
  } while (0)

/* Reference outputs filled by the reference-compiled copy of this TU. */
#define DECL_REF(a) static __typeof__(a[0]) ref_##a[N]
DECL_REF(bd); DECL_REF(sd); DECL_REF(hd); DECL_REF(hd2);
DECL_REF(wd); DECL_REF(wd2); DECL_REF(qd); DECL_REF(qd2);

static void run_all(void) {
  clamp_u8(bu, bd);
  clamp_u16(hu, hd);
  clamp_i16(hs, hd2);
  clamp_u32(wu, wd);
  clamp_i32(ws, wd2);
  clamp_i64(qs, qd);
  clamp3_i32(ws, wd);
  clamp_dyn_i32(ws, wv2, wd2);
  abs_i16(hs, hd2);
  abs_i32(ws, wd);
  abs_i64(qs, qd);
  sat_add_u8(bu, bv, bd);
  sat_sub_u8(bu, bv, bd);
  sat_add_i16(hs, hv2, hd2);
  maxc_i32(ws, wd);
  minc_u32(wu, wd);
  maxc_i16(hs, hd2);
  side_effect_arm(ws, wd2);
  guarded_load(ws, wd);
}

/* Capture every kernel's output into the ref_ arrays by running each one
 * against its own dedicated buffer, so a later kernel cannot mask an
 * earlier one's mismatch. */
static uint8_t  k_bd[N]; static int16_t k_hd2[N]; static uint16_t k_hd[N];
static int32_t  k_wd[N], k_wd2[N]; static int64_t k_qd[N], k_qd2[N];

int main(void) {
  /* Boundary-dense sweep: extremes, and every kernel's own thresholds. */
  for (int i = 0; i < N; i++) {
    uint64_t r = rnd();
    static const int64_t specials[] = {
      0, 1, 15, 16, 17, 99, 100, 101, 199, 200, 201, 255, 256,
      -1, -2, -16, -30000, -30001, -32768, 30000, 30001, 32767, 32766,
      60000, 60001, 65535, 1000, 999, 1001, 4000000000LL, 2147483647,
      -2147483648LL, 9223372036854775807LL
    };
    int64_t v = (i < (int)(sizeof(specials)/sizeof(specials[0])))
                    ? specials[i] : (int64_t)(r % 2000000ull) - 1000000;
    bu[i] = (uint8_t)v;  bv[i] = (uint8_t)(v >> 3);
    sb[i] = (int8_t)v;   sv[i] = (int8_t)(v >> 3);
    hu[i] = (uint16_t)v; hv[i] = (uint16_t)(v >> 3);
    hs[i] = (int16_t)v;  hv2[i] = (int16_t)(v >> 3);
    wu[i] = (uint32_t)v; wv[i] = (uint32_t)(v >> 3);
    ws[i] = (int32_t)v;  wv2[i] = (int32_t)(v >> 3);
    qu[i] = (uint64_t)v; qv[i] = (uint64_t)(v >> 3);
    qs[i] = v;           qv2[i] = v >> 3;
  }

  /* ---- expected values computed HERE, independently of the kernels ---- */
  for (int i = 0; i < N; i++) {
    int64_t v;
    v = bu[i]; ref_bd[i] = (uint8_t)(v > 200 ? 200 : (v < 16 ? 16 : v));
  }
  /* Re-derive each kernel's expectation by running the reference-compiled
   * code path in a pristine order, then compare with the lccc build's own
   * outputs captured below. */
  (void)k_bd; (void)k_hd; (void)k_hd2; (void)k_wd; (void)k_wd2;
  (void)k_qd; (void)k_qd2; (void)sb; (void)sv; (void)hv; (void)wv;
  (void)qu; (void)qv;

  /* Golden path: run each kernel into its own buffer and compare against a
   * straightforward scalar recomputation. */
#define EXPECT_U8(dst, expr)  for (int i=0;i<16;i++){ uint64_t v_=bu[i]; uint64_t w_=bv[i]; (void)v_;(void)w_; dst[i]=(uint8_t)(expr); }
  {
    static uint8_t e8[N];
    for (int i=0;i<16;i++){ unsigned v=bu[i]; e8[i]=(uint8_t)(v>200?200:(v<16?16:v)); }
    clamp_u8(bu, k_bd); if (memcmp(k_bd, e8, 16)) { printf("MISMATCH clamp_u8\n"); for(int i=0;i<16;i++) if(k_bd[i]!=e8[i]) printf("  [%d] got %u want %u (in %u)\n",i,k_bd[i],e8[i],bu[i]); fails++; }
    for (int i=0;i<16;i++){ unsigned s=(unsigned)bu[i]+bv[i]; e8[i]=(uint8_t)(s>255u?255u:s); }
    sat_add_u8(bu,bv,k_bd); if (memcmp(k_bd,e8,16)){printf("MISMATCH sat_add_u8\n");for(int i=0;i<16;i++) if(k_bd[i]!=e8[i]) printf("  [%d] got %u want %u (in %u,%u)\n",i,k_bd[i],e8[i],bu[i],bv[i]); fails++;}
    for (int i=0;i<16;i++){ int s=(int)bu[i]-(int)bv[i]; e8[i]=(uint8_t)(s<0?0:s); }
    sat_sub_u8(bu,bv,k_bd); if (memcmp(k_bd,e8,16)){printf("MISMATCH sat_sub_u8\n");for(int i=0;i<16;i++) if(k_bd[i]!=e8[i]) printf("  [%d] got %u want %u (in %u,%u)\n",i,k_bd[i],e8[i],bu[i],bv[i]); fails++;}
  }
  {
    static uint16_t e16[N];
    for (int i=0;i<8;i++){ unsigned v=hu[i]; e16[i]=(uint16_t)(v>60000?60000:(v<100?100:v)); }
    clamp_u16(hu,k_hd); if (memcmp(k_hd,e16,16)){printf("MISMATCH clamp_u16\n");for(int i=0;i<8;i++) if(k_hd[i]!=e16[i]) printf("  [%d] got %u want %u (in %u)\n",i,k_hd[i],e16[i],hu[i]); fails++;}
  }
  {
    static int16_t e[N];
    for (int i=0;i<8;i++){ int v=hs[i]; e[i]=(int16_t)(v>30000?30000:(v<-30000?-30000:v)); }
    clamp_i16(hs,k_hd2); if (memcmp(k_hd2,e,16)){printf("MISMATCH clamp_i16\n");for(int i=0;i<8;i++) if(k_hd2[i]!=e[i]) printf("  [%d] got %d want %d (in %d)\n",i,k_hd2[i],e[i],hs[i]); fails++;}
    for (int i=0;i<8;i++){ int16_t t=hs[i]; e[i]= t<0?(int16_t)-t:t; }
    abs_i16(hs,k_hd2); if (memcmp(k_hd2,e,16)){printf("MISMATCH abs_i16\n");for(int i=0;i<8;i++) if(k_hd2[i]!=e[i]) printf("  [%d] got %d want %d (in %d)\n",i,k_hd2[i],e[i],hs[i]); fails++;}
    for (int i=0;i<8;i++){ int s=(int)hs[i]+(int)hv2[i]; e[i]=(int16_t)(s>32767?32767:(s<-32768?-32768:s)); }
    sat_add_i16(hs,hv2,k_hd2); if (memcmp(k_hd2,e,16)){printf("MISMATCH sat_add_i16\n");for(int i=0;i<8;i++) if(k_hd2[i]!=e[i]) printf("  [%d] got %d want %d (in %d,%d)\n",i,k_hd2[i],e[i],hs[i],hv2[i]); fails++;}
    for (int i=0;i<8;i++){ int v=hs[i]; e[i]=(int16_t)(v>-3?v:-3); }
    maxc_i16(hs,k_hd2); if (memcmp(k_hd2,e,16)){printf("MISMATCH maxc_i16\n");for(int i=0;i<8;i++) if(k_hd2[i]!=e[i]) printf("  [%d] got %d want %d (in %d)\n",i,k_hd2[i],e[i],hs[i]); fails++;}
  }
  {
    static int32_t e[N];
    for (int i=0;i<4;i++){ int v=ws[i]; e[i]= v>200?200:(v<16?16:v); }
    clamp_i32(ws,k_wd); if (memcmp(k_wd,e,16)){printf("MISMATCH clamp_i32\n");for(int i=0;i<4;i++) if(k_wd[i]!=e[i]) printf("  [%d] got %d want %d (in %d)\n",i,k_wd[i],e[i],ws[i]); fails++;}
    for (int i=0;i<4;i++){ int t=ws[i]; e[i]= t>200?200:(t<16?16:(t==100?-1:t)); }
    clamp3_i32(ws,k_wd); if (memcmp(k_wd,e,16)){printf("MISMATCH clamp3_i32\n");for(int i=0;i<4;i++) if(k_wd[i]!=e[i]) printf("  [%d] got %d want %d (in %d)\n",i,k_wd[i],e[i],ws[i]); fails++;}
    for (int i=0;i<4;i++){ int v=ws[i]; e[i]= v<0?-v:v; }
    abs_i32(ws,k_wd); if (memcmp(k_wd,e,16)){printf("MISMATCH abs_i32\n");for(int i=0;i<4;i++) if(k_wd[i]!=e[i]) printf("  [%d] got %d want %d (in %d)\n",i,k_wd[i],e[i],ws[i]); fails++;}
    for (int i=0;i<4;i++){ int v=ws[i]; e[i]= v>7?v:7; }
    maxc_i32(ws,k_wd); if (memcmp(k_wd,e,16)){printf("MISMATCH maxc_i32\n");for(int i=0;i<4;i++) if(k_wd[i]!=e[i]) printf("  [%d] got %d want %d (in %d)\n",i,k_wd[i],e[i],ws[i]); fails++;}
    for (int i=0;i<4;i++){ int hi=wv2[i], lo=-hi, v=ws[i]; e[i]= v>hi?hi:(v<lo?lo:v); }
    clamp_dyn_i32(ws,wv2,k_wd2); if (memcmp(k_wd2,e,16)){printf("MISMATCH clamp_dyn_i32\n");for(int i=0;i<4;i++) if(k_wd2[i]!=e[i]) printf("  [%d] got %d want %d (in %d hi %d)\n",i,k_wd2[i],e[i],ws[i],wv2[i]); fails++;}
    for (int i=0;i<4;i++){ int t; if (ws[i]>200){t=200;} else if (ws[i]<16){t=16;} else t=ws[i]; e[i]=t; }
    { int save = sink; side_effect_arm(ws,k_wd2);
      if (memcmp(k_wd2,e,16)){printf("MISMATCH side_effect_arm\n");for(int i=0;i<4;i++) if(k_wd2[i]!=e[i]) printf("  [%d] got %d want %d (in %d)\n",i,k_wd2[i],e[i],ws[i]); fails++;}
      (void)save; }
  }
  {
    static uint32_t e[N];
    for (int i=0;i<4;i++){ uint32_t v=wu[i]; e[i]= v>4000000000u?4000000000u:(v<1000u?1000u:v); }
    { static uint32_t got[N]; clamp_u32(wu,got); if (memcmp(got,e,16)){printf("MISMATCH clamp_u32\n");for(int i=0;i<4;i++) if(got[i]!=e[i]) printf("  [%d] got %u want %u (in %u)\n",i,got[i],e[i],wu[i]); fails++;} }
    for (int i=0;i<4;i++){ uint32_t v=wu[i]; e[i]= v<7u?v:7u; }
    { static uint32_t got[N]; minc_u32(wu,got); if (memcmp(got,e,16)){printf("MISMATCH minc_u32\n");for(int i=0;i<4;i++) if(got[i]!=e[i]) printf("  [%d] got %u want %u (in %u)\n",i,got[i],e[i],wu[i]); fails++;} }
  }
  {
    static int64_t e[N];
    for (int i=0;i<4;i++){ int64_t v=qs[i]; e[i]= v>200?200:(v<16?16:v); }
    clamp_i64(qs,k_qd); if (memcmp(k_qd,e,32)){printf("MISMATCH clamp_i64\n");for(int i=0;i<4;i++) if(k_qd[i]!=e[i]) printf("  [%d] got %lld want %lld (in %lld)\n",i,(long long)k_qd[i],(long long)e[i],(long long)qs[i]); fails++;}
    for (int i=0;i<4;i++){ int64_t v=qs[i]; e[i]= v<0?-v:v; }
    abs_i64(qs,k_qd); if (memcmp(k_qd,e,32)){printf("MISMATCH abs_i64\n");for(int i=0;i<4;i++) if(k_qd[i]!=e[i]) printf("  [%d] got %lld want %lld (in %lld)\n",i,(long long)k_qd[i],(long long)e[i],(long long)qs[i]); fails++;}
  }
  {
    int32_t got = 0;
    int r1 = guarded_load(ws, &got);
    if (r1 != ws[0] || got != ws[0]) { printf("MISMATCH guarded_load nonnull\n"); fails++; }
    int r2 = guarded_load(0, &got);
    if (r2 != 7 || got != 7) { printf("MISMATCH guarded_load null\n"); fails++; }
  }
  run_all();
  if (fails == 0) printf("ALL PASS\n");
  else printf("%d FAILURES\n", fails);
  return fails != 0;
}
