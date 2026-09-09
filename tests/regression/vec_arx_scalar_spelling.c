/*
 * ARX lane vectorization, PHI-spelling gate (PR #455 port).
 *
 * The memory-form pass (`tests/benchmark/programs/chacha20_block.c` plus
 * `vec_arx`) recognizes the `u32 x[16]` ARRAY spelling; this file gates the
 * complementary PHI spelling: 16 SCALAR u32 locals (a..p) promoted to SSA
 * loop-header phis by mem2reg.  The pass must either rewrite the loop to
 * the 4-lane SIMD double round EXACTLY or decline and leave the scalar
 * form — both outcomes produce the same bits.
 *
 * Kernels:
 *   chacha20_core  — RFC 7539 section 2.3.2 known-answer vector, full
 *                    16-word check (rotate schedule 16/12/8/7 in BOTH
 *                    groups, column round + diagonal round).  The
 *                    transform is expected to fire here at -O2+.
 *   blake_core     — same wiring, second group's schedule swapped to the
 *                    BLAKE-style 8/7/16/12: the symbolic proof derives the
 *                    schedule from the terms, so this must vectorize (and
 *                    be exact) as well.
 *   cycled_core    — REFUSAL shape: Salsa-style role-cycling wiring (the
 *                    diagonal round's a/b operands swapped), outside the
 *                    lane-parallel class.  Correct scalar result required.
 *   twelve_core    — REFUSAL shape: a 12-word state (phi census demands
 *                    exactly 16 words + the counter).  Correct scalar
 *                    result required.
 *   arot_core      — REFUSAL shape: a rotate on the A role (the class
 *                    rotates only b and d).  Correct scalar result
 *                    required.
 *
 * Expected values are absolute constants (RFC 7539 test vector + reference
 * evaluation), so any correct compiler — LCCC at any opt level, GCC — must
 * reproduce them bit-for-bit.
 */
#include <stdio.h>

typedef unsigned int u32;

#define ROTL32(v, n) (((v) << (n)) | ((v) >> (32 - (n))))

#define QR(a, b, c, d) \
  do { \
    a += b; d ^= a; d = ROTL32(d, 16); \
    c += d; b ^= c; b = ROTL32(b, 12); \
    a += b; d ^= a; d = ROTL32(d, 8);  \
    c += d; b ^= c; b = ROTL32(b, 7);  \
  } while (0)

/* BLAKE-style second group: schedule 8/7/16/12. */
#define QR_B(a, b, c, d) \
  do { \
    a += b; d ^= a; d = ROTL32(d, 8);  \
    c += d; b ^= c; b = ROTL32(b, 7);  \
    a += b; d ^= a; d = ROTL32(d, 16); \
    c += d; b ^= c; b = ROTL32(b, 12); \
  } while (0)

/* Rotate on the A role (outside the class: only b/d rotate). */
#define QR_A(a, b, c, d) \
  do { \
    a += b; a = ROTL32(a, 13); \
    d ^= a; d = ROTL32(d, 16); \
    c += d; b ^= c; b = ROTL32(b, 12); \
    a += b; d ^= a; d = ROTL32(d, 8);  \
    c += d; b ^= c; b = ROTL32(b, 7);  \
  } while (0)

static void
chacha20_core(u32 out[16], const u32 in[16])
{
  u32 a, b, c, d, e, f, g, h, i, j, k, l, m, n, o, p;
  int t;

  a = in[0];  b = in[1];  c = in[2];  d = in[3];
  e = in[4];  f = in[5];  g = in[6];  h = in[7];
  i = in[8];  j = in[9];  k = in[10]; l = in[11];
  m = in[12]; n = in[13]; o = in[14]; p = in[15];

  for (t = 0; t < 10; t++) {
    /* Column round: lane-aligned group. */
    QR(a, e, i, m); QR(b, f, j, n); QR(c, g, k, o); QR(d, h, l, p);
    /* Diagonal round: lane-rotated group (offsets 1, 2, 3). */
    QR(a, f, k, p); QR(b, g, l, m); QR(c, h, i, n); QR(d, e, j, o);
  }

  out[0]  = a + in[0];  out[1]  = b + in[1];  out[2]  = c + in[2];  out[3]  = d + in[3];
  out[4]  = e + in[4];  out[5]  = f + in[5];  out[6]  = g + in[6];  out[7]  = h + in[7];
  out[8]  = i + in[8];  out[9]  = j + in[9];  out[10] = k + in[10]; out[11] = l + in[11];
  out[12] = m + in[12]; out[13] = n + in[13]; out[14] = o + in[14]; out[15] = p + in[15];
}

static void
blake_core(u32 out[16], const u32 in[16])
{
  u32 a, b, c, d, e, f, g, h, i, j, k, l, m, n, o, p;
  int t;

  a = in[0];  b = in[1];  c = in[2];  d = in[3];
  e = in[4];  f = in[5];  g = in[6];  h = in[7];
  i = in[8];  j = in[9];  k = in[10]; l = in[11];
  m = in[12]; n = in[13]; o = in[14]; p = in[15];

  for (t = 0; t < 10; t++) {
    /* Column round: standard schedule. */
    QR(a, e, i, m); QR(b, f, j, n); QR(c, g, k, o); QR(d, h, l, p);
    /* Diagonal round: BLAKE-style swapped schedule 8/7/16/12. */
    QR_B(a, f, k, p); QR_B(b, g, l, m); QR_B(c, h, i, n); QR_B(d, e, j, o);
  }

  out[0]  = a + in[0];  out[1]  = b + in[1];  out[2]  = c + in[2];  out[3]  = d + in[3];
  out[4]  = e + in[4];  out[5]  = f + in[5];  out[6]  = g + in[6];  out[7]  = h + in[7];
  out[8]  = i + in[8];  out[9]  = j + in[9];  out[10] = k + in[10]; out[11] = l + in[11];
  out[12] = m + in[12]; out[13] = n + in[13]; out[14] = o + in[14]; out[15] = p + in[15];
}

static void
cycled_core(u32 out[16], const u32 in[16])
{
  u32 a, b, c, d, e, f, g, h, i, j, k, l, m, n, o, p;
  int t;

  a = in[0];  b = in[1];  c = in[2];  d = in[3];
  e = in[4];  f = in[5];  g = in[6];  h = in[7];
  i = in[8];  j = in[9];  k = in[10]; l = in[11];
  m = in[12]; n = in[13]; o = in[14]; p = in[15];

  for (t = 0; t < 10; t++) {
    QR(a, e, i, m); QR(b, f, j, n); QR(c, g, k, o); QR(d, h, l, p);
    /* Salsa-style role-cycling: the a/b operands of each diagonal round
       are swapped, so the second group's a-words come from the B role —
       not a lane-parallel wiring.  The proof must decline. */
    QR(f, a, k, p); QR(g, b, l, m); QR(h, c, i, n); QR(e, d, j, o);
  }

  out[0]  = a + in[0];  out[1]  = b + in[1];  out[2]  = c + in[2];  out[3]  = d + in[3];
  out[4]  = e + in[4];  out[5]  = f + in[5];  out[6]  = g + in[6];  out[7]  = h + in[7];
  out[8]  = i + in[8];  out[9]  = j + in[9];  out[10] = k + in[10]; out[11] = l + in[11];
  out[12] = m + in[12]; out[13] = n + in[13]; out[14] = o + in[14]; out[15] = p + in[15];
}

static void
twelve_core(u32 out[12], const u32 in[12])
{
  u32 a, b, c, d, e, f, g, h, i, j, k, l;
  int t;

  a = in[0];  b = in[1];  c = in[2];
  d = in[3];  e = in[4];  f = in[5];
  g = in[6];  h = in[7];  i = in[8];
  j = in[9];  k = in[10]; l = in[11];

  for (t = 0; t < 10; t++) {
    QR(a, d, g, j); QR(b, e, h, k); QR(c, f, i, l);
    QR(a, e, i, l); QR(b, f, g, j); QR(c, d, h, k);
  }

  out[0]  = a + in[0];  out[1]  = b + in[1];  out[2]  = c + in[2];
  out[3]  = d + in[3];  out[4]  = e + in[4];  out[5]  = f + in[5];
  out[6]  = g + in[6];  out[7]  = h + in[7];  out[8]  = i + in[8];
  out[9]  = j + in[9];  out[10] = k + in[10]; out[11] = l + in[11];
}

static void
arot_core(u32 out[16], const u32 in[16])
{
  u32 a, b, c, d, e, f, g, h, i, j, k, l, m, n, o, p;
  int t;

  a = in[0];  b = in[1];  c = in[2];  d = in[3];
  e = in[4];  f = in[5];  g = in[6];  h = in[7];
  i = in[8];  j = in[9];  k = in[10]; l = in[11];
  m = in[12]; n = in[13]; o = in[14]; p = in[15];

  for (t = 0; t < 10; t++) {
    QR_A(a, e, i, m); QR_A(b, f, j, n); QR_A(c, g, k, o); QR_A(d, h, l, p);
    QR_A(a, f, k, p); QR_A(b, g, l, m); QR_A(c, h, i, n); QR_A(d, e, j, o);
  }

  out[0]  = a + in[0];  out[1]  = b + in[1];  out[2]  = c + in[2];  out[3]  = d + in[3];
  out[4]  = e + in[4];  out[5]  = f + in[5];  out[6]  = g + in[6];  out[7]  = h + in[7];
  out[8]  = i + in[8];  out[9]  = j + in[9];  out[10] = k + in[10]; out[11] = l + in[11];
  out[12] = m + in[12]; out[13] = n + in[13]; out[14] = o + in[14]; out[15] = p + in[15];
}

/* RFC 7539 section 2.3.2 block-function test vector (with feed-forward). */
static const u32 test_in[16] = {
  0x61707865, 0x3320646e, 0x79622d32, 0x6b206574,
  0x03020100, 0x07060504, 0x0b0a0908, 0x0f0e0d0c,
  0x13121110, 0x17161514, 0x1b1a1918, 0x1f1e1d1c,
  0x00000001, 0x09000000, 0x4a000000, 0x00000000
};
static const u32 in12[12] = {
  0x61707865, 0x3320646e, 0x79622d32, 0x03020100,
  0x07060504, 0x0b0a0908, 0x13121110, 0x17161514,
  0x1b1a1918, 0x00000001, 0x09000000, 0x4a000000
};

static int fails = 0;

static void
check16(const char *name, const u32 *got, const u32 *want)
{
  int i;
  for (i = 0; i < 16; i++) {
    if (got[i] != want[i]) {
      printf("FAIL %s: out[%d] = %08x, want %08x\n", name, i, got[i], want[i]);
      fails++;
    }
  }
}

int
main(void)
{
  u32 out[16], out12[12];

  chacha20_core(out, test_in);
  check16("chacha20_core", out, (const u32[16]) {
    0xe4e7f110, 0x15593bd1, 0x1fdd0f50, 0xc47120a3,
    0xc7f4d1c7, 0x0368c033, 0x9aaa2204, 0x4e6cd4c3,
    0x466482d2, 0x09aa9f07, 0x05d7c214, 0xa2028bd9,
    0xd19c12b5, 0xb94e16de, 0xe883d0cb, 0x4e3c50a2
  });

  blake_core(out, test_in);
  check16("blake_core", out, (const u32[16]) {
    0xe0a871ec, 0x299500f8, 0x0f244d4f, 0x484cce1e,
    0x55fb7667, 0x3055285e, 0x0b5cd60b, 0x8f684b9e,
    0x7e27e3f1, 0xa3e5dac7, 0x29126bac, 0x7fa87974,
    0x1a6e9a80, 0xb38ceefa, 0x2accbca7, 0xb38a6668
  });

  cycled_core(out, test_in);
  check16("cycled_core", out, (const u32[16]) {
    0x5de1c4ab, 0x1d7b7eca, 0x84cc6cb4, 0xb52272d4,
    0xc0a7fabc, 0xe52535a5, 0xb1c57fca, 0x198fd35c,
    0xd559018a, 0x0b17817e, 0x9ab9f3bb, 0x2d0fd28f,
    0x1f9c8e35, 0xf510c97e, 0xe631c2e4, 0xdbe78ede
  });

  twelve_core(out12, in12);
  {
    int i;
    const u32 want[12] = {
      0x7cca04d8, 0xc0f71e6f, 0x9d2254a7, 0xc4969b79,
      0x8cb53cb0, 0xc2204481, 0x3d628563, 0xfca9124f,
      0x538eddfe, 0xf71c1416, 0xa5324099, 0x14a84825
    };
    for (i = 0; i < 12; i++) {
      if (out12[i] != want[i]) {
        printf("FAIL twelve_core: out[%d] = %08x, want %08x\n", i, out12[i], want[i]);
        fails++;
      }
    }
  }

  arot_core(out, test_in);
  check16("arot_core", out, (const u32[16]) {
    0x846b8575, 0x1964706b, 0x4484f6e4, 0x1e4b2b1b,
    0xf6211736, 0x378c87b8, 0x8f46ad07, 0x7eddaf2a,
    0x31aa6d29, 0x9a004e2f, 0x6bed7de2, 0xc25d3e86,
    0x47978018, 0x6ab24806, 0x1fae4bb4, 0x17936c16
  });

  if (fails) {
    printf("VALIDATION FAILED (%d mismatches)\n", fails);
    return 1;
  }
  puts("VALIDATION OK");
  return 0;
}
