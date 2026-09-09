/*
 * Workload-derived kernel: SHA-256 compression function (FIPS 180-4).
 *
 * Used extensively across Linux kernel (crypto/sha256_generic.c), OpenSSL,
 * Git (object hashing), and system utilities.
 *
 * This kernel stresses 32-bit rotate right operations (ror), 64-step message
 * schedule expansion, unrolling, register allocation across 8 working
 * variables (a-h), and instruction scheduling.
 */
#include <stdio.h>

#ifndef BLOCK_COUNT
#define BLOCK_COUNT (1U << 17)
#endif
#ifndef PASSES
#define PASSES 8U
#endif

typedef unsigned int u32;
typedef unsigned long long u64;

#define ROR32(x, n) (((x) >> (n)) | ((x) << (32 - (n))))

#define CH(x, y, z)  (((x) & (y)) ^ (~(x) & (z)))
#define MAJ(x, y, z) (((x) & (y)) ^ ((x) & (z)) ^ ((y) & (z)))
#define EP0(x)       (ROR32(x, 2) ^ ROR32(x, 13) ^ ROR32(x, 22))
#define EP1(x)       (ROR32(x, 6) ^ ROR32(x, 11) ^ ROR32(x, 25))
#define SIG0(x)      (ROR32(x, 7) ^ ROR32(x, 18) ^ ((x) >> 3))
#define SIG1(x)      (ROR32(x, 17) ^ ROR32(x, 19) ^ ((x) >> 10))

static const u32 K[64] = {
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5,
  0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
  0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
  0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
  0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc,
  0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
  0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
  0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
  0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
  0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3,
  0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5,
  0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
  0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
  0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2
};

static void
sha256_transform(u32 state[8], const u32 data[16])
{
  u32 a, b, c, d, e, f, g, h, t1, t2, m[64];
  int i;

  for (i = 0; i < 16; ++i)
    m[i] = data[i];
  for (i = 16; i < 64; ++i)
    m[i] = SIG1(m[i - 2]) + m[i - 7] + SIG0(m[i - 15]) + m[i - 16];

  a = state[0];
  b = state[1];
  c = state[2];
  d = state[3];
  e = state[4];
  f = state[5];
  g = state[6];
  h = state[7];

  for (i = 0; i < 64; ++i) {
    t1 = h + EP1(e) + CH(e, f, g) + K[i] + m[i];
    t2 = EP0(a) + MAJ(a, b, c);
    h = g;
    g = f;
    f = e;
    e = d + t1;
    d = c;
    c = b;
    b = a;
    a = t1 + t2;
  }

  state[0] += a;
  state[1] += b;
  state[2] += c;
  state[3] += d;
  state[4] += e;
  state[5] += f;
  state[6] += g;
  state[7] += h;
}

static int
check_known_vector(void)
{
  /* Standard SHA-256 test on single block "abc" padded */
  u32 state[8] = {
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
    0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19
  };
  u32 data[16] = {
    0x61626380, 0x00000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000, 0x00000018
  };
  sha256_transform(state, data);
  if (state[0] != 0xba7816bf || state[1] != 0x8f01cfea ||
      state[7] != 0xf20015ad)
    return 0;
  return 1;
}

int
main(void)
{
  u32 state[8];
  u32 data[16];
  u64 checksum = 0U;
  unsigned int pass, b;
  int i;

  if (!check_known_vector())
    return 2;

  for (pass = 0; pass < PASSES; pass++) {
    state[0] = 0x6a09e667 ^ pass;
    state[1] = 0xbb67ae85;
    state[2] = 0x3c6ef372;
    state[3] = 0xa54ff53a;
    state[4] = 0x510e527f;
    state[5] = 0x9b05688c;
    state[6] = 0x1f83d9ab;
    state[7] = 0x5be0cd19;

    for (b = 0; b < BLOCK_COUNT; b++) {
      for (i = 0; i < 16; i++)
        data[i] = (u32)(b * 1664525U + (u32)i * 1013904223U) ^ state[i & 7];
      sha256_transform(state, data);
      checksum += ((u64)state[0] << 32) ^ state[7] ^ ((u64)state[3] << 16);
    }
  }

  printf("%016llx\n", checksum);
  return 0;
}
