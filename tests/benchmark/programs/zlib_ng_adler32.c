/*
 * Workload-derived kernel: zlib-ng 2.3.3, arch/generic/adler32_c.c and
 * adler32_p.h.
 *
 * SPDX-License-Identifier: Zlib
 *
 * The checksum kernel is a standalone adaptation of zlib-ng's generic
 * Adler-32 implementation.  Dispatch, project-private headers, and copy
 * variants were removed; the NMAX blocking and eight-byte accumulation shape
 * are retained.  See tests/benchmark/WORKLOAD_PROVENANCE.md for the pinned
 * Arch recipe, upstream tag, artifact hash, and adaptation boundaries.
 *
 * It stresses unrolled scalar loads, two dependent accumulation chains,
 * modulo lowering, loop formation, and register allocation.
 */
#include <stdio.h>

#define BASE 65521U
#define NMAX 5552U
#define DATA_SIZE (2UL << 20)
#define PASSES 48U

#define ADLER_DO1(sum1, sum2, buf, i) \
  { (sum1) += (buf)[(i)]; (sum2) += (sum1); }
#define ADLER_DO2(sum1, sum2, buf, i) \
  { ADLER_DO1(sum1, sum2, buf, i); ADLER_DO1(sum1, sum2, buf, (i) + 1); }
#define ADLER_DO4(sum1, sum2, buf, i) \
  { ADLER_DO2(sum1, sum2, buf, i); ADLER_DO2(sum1, sum2, buf, (i) + 2); }
#define ADLER_DO8(sum1, sum2, buf, i) \
  { ADLER_DO4(sum1, sum2, buf, i); ADLER_DO4(sum1, sum2, buf, (i) + 4); }

static unsigned char zlib_ng_adler_data[DATA_SIZE];

static unsigned int
zlib_ng_adler32_len_16(unsigned int sum1, const unsigned char *buf,
                       unsigned long len, unsigned int sum2)
{
  while (len) {
    --len;
    sum1 += *buf++;
    sum2 += sum1;
  }
  sum1 %= BASE;
  sum2 %= BASE;
  return sum1 | (sum2 << 16);
}

static unsigned int
zlib_ng_adler32_len_64(unsigned int sum1, const unsigned char *buf,
                       unsigned long len, unsigned int sum2)
{
  while (len >= 8U) {
    len -= 8U;
    ADLER_DO8(sum1, sum2, buf, 0);
    buf += 8;
  }
  return zlib_ng_adler32_len_16(sum1, buf, len, sum2);
}

/* Derived directly from zlib-ng's arch/generic/adler32_c.c. */
static unsigned int
zlib_ng_adler32_c(unsigned int adler, const unsigned char *buf,
                  unsigned long len)
{
  unsigned int sum2;
  unsigned int n;

  sum2 = (adler >> 16) & 0xffffU;
  adler &= 0xffffU;

  if (len == 1U)
    return zlib_ng_adler32_len_16(adler, buf, 1U, sum2);
  if (buf == 0)
    return 1U;
  if (len < 16U)
    return zlib_ng_adler32_len_16(adler, buf, len, sum2);

  while (len >= NMAX) {
    len -= NMAX;
    n = NMAX / 8U;
    do {
      ADLER_DO8(adler, sum2, buf, 0);
      buf += 8;
    } while (--n);
    adler %= BASE;
    sum2 %= BASE;
  }

  return zlib_ng_adler32_len_64(adler, buf, len, sum2);
}

static void
fill_data(void)
{
  unsigned long i;
  unsigned int state = 0x9e3779b9U;

  for (i = 0; i < DATA_SIZE; i++) {
    state = state * 1103515245U + 12345U;
    zlib_ng_adler_data[i] = (unsigned char)(state >> 16);
  }
}

int
main(void)
{
  static const unsigned char check[] = "123456789";
  unsigned int pass;
  unsigned int checksum = 0U;

  /* Adler-32's standard check vector. */
  if (zlib_ng_adler32_c(1U, check, 9U) != 0x091e01deU)
    return 2;

  fill_data();
  for (pass = 0; pass < PASSES; pass++) {
    unsigned int adler = zlib_ng_adler32_c(1U + pass,
                                            zlib_ng_adler_data, DATA_SIZE);
    checksum ^= adler + pass;
    /* Preserve the valid input domain while preventing loop hoisting. */
    zlib_ng_adler_data[(pass * 12289U) & (DATA_SIZE - 1)] ^=
        (unsigned char)(adler >> 9);
  }

  printf("%08x\n", checksum);
  return 0;
}
