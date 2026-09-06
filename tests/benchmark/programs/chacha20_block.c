/*
 * Workload-derived kernel: ChaCha20 block function / ARX permutation.
 *
 * ChaCha20 is widely used in the Linux kernel (random/crypto), OpenSSL,
 * BoringSSL, and libsodium (RFC 7539).
 *
 * This kernel stresses 32-bit integer arithmetic (ARX: Add-Rotate-Xor),
 * rotate-instruction selection (roll), 16-register pressure, unrolling, and
 * vectorization/instruction-level parallelism across 4 parallel quarter rounds.
 */
#include <stdio.h>

#define BLOCK_COUNT (1U << 17)
#define PASSES 16U

typedef unsigned int u32;
typedef unsigned long long u64;

#define ROTL32(v, n) (((v) << (n)) | ((v) >> (32 - (n))))

#define QR(a, b, c, d) \
  do { \
    a += b; d ^= a; d = ROTL32(d, 16); \
    c += d; b ^= c; b = ROTL32(b, 12); \
    a += b; d ^= a; d = ROTL32(d, 8);  \
    c += d; b ^= c; b = ROTL32(b, 7);  \
  } while (0)

static void
chacha20_core(u32 out[16], const u32 in[16])
{
  u32 x[16];
  int i;

  for (i = 0; i < 16; i++)
    x[i] = in[i];

  for (i = 0; i < 10; i++) {
    /* Column round */
    QR(x[0], x[4], x[8],  x[12]);
    QR(x[1], x[5], x[9],  x[13]);
    QR(x[2], x[6], x[10], x[14]);
    QR(x[3], x[7], x[11], x[15]);

    /* Diagonal round */
    QR(x[0], x[5], x[10], x[15]);
    QR(x[1], x[6], x[11], x[12]);
    QR(x[2], x[7], x[8],  x[13]);
    QR(x[3], x[4], x[9],  x[14]);
  }

  for (i = 0; i < 16; i++)
    out[i] = x[i] + in[i];
}

static int
check_known_vector(void)
{
  /* RFC 7539 section 2.3.2 test vector */
  static const u32 test_in[16] = {
    0x61707865, 0x3320646e, 0x79622d32, 0x6b206574,
    0x03020100, 0x07060504, 0x0b0a0908, 0x0f0e0d0c,
    0x13121110, 0x17161514, 0x1b1a1918, 0x1f1e1d1c,
    0x00000001, 0x09000000, 0x4a000000, 0x00000000
  };
  u32 test_out[16];
  chacha20_core(test_out, test_in);
  if (test_out[0] != 0xe4e7f110 || test_out[1] != 0x15593bd1 ||
      test_out[14] != 0xe883d0cb || test_out[15] != 0x4e3c50a2)
    return 0;
  return 1;
}

int
main(void)
{
  u32 in[16];
  u32 out[16];
  u64 checksum = 0U;
  unsigned int pass;
  unsigned int b;
  int i;

  if (!check_known_vector())
    return 2;

  /* Initialize key, constant, nonce state */
  in[0] = 0x61707865; in[1] = 0x3320646e; in[2] = 0x79622d32; in[3] = 0x6b206574;
  for (i = 4; i < 12; i++)
    in[i] = 0x01010101U * (u32)i;
  in[12] = 0;
  in[13] = 0xdeadbeefU;
  in[14] = 0x01234567U;
  in[15] = 0x89abcdefU;

  for (pass = 0; pass < PASSES; pass++) {
    for (b = 0; b < BLOCK_COUNT; b++) {
      in[12] = b ^ pass;
      chacha20_core(out, in);
      checksum += ((u64)out[0] << 32) ^ out[15] ^ ((u64)out[7] << 16);
      in[4] ^= out[0] & 0xffU;
    }
  }

  printf("%016llx\n", checksum);
  return 0;
}
