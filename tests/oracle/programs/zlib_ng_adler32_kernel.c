/* Oracle reproducer (tests/oracle/delta_corpus.json): zlib-ng arch/generic/adler32_c.c (ADLER_DO8 + NMAX blocking)
 * Extracted verbatim from tests/benchmark/programs/zlib_ng_adler32.c; the measured
 * function is exported so no compiler can inline it away.  Provenance:
 * tests/benchmark/WORKLOAD_PROVENANCE.md. */
#define BASE 65521U
#define NMAX 5552U

#define ADLER_DO1(sum1, sum2, buf, i) \
  { (sum1) += (buf)[(i)]; (sum2) += (sum1); }
#define ADLER_DO2(sum1, sum2, buf, i) \
  { ADLER_DO1(sum1, sum2, buf, i); ADLER_DO1(sum1, sum2, buf, (i) + 1); }
#define ADLER_DO4(sum1, sum2, buf, i) \
  { ADLER_DO2(sum1, sum2, buf, i); ADLER_DO2(sum1, sum2, buf, (i) + 2); }
#define ADLER_DO8(sum1, sum2, buf, i) \
  { ADLER_DO4(sum1, sum2, buf, i); ADLER_DO4(sum1, sum2, buf, (i) + 4); }

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
unsigned int
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
