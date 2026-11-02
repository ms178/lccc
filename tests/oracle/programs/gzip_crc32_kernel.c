/* Oracle reproducer (tests/oracle/delta_corpus.json): GNU gzip lib/crc.c scalar table loop
 * Extracted verbatim from tests/benchmark/programs/gzip_crc32.c; the measured
 * functions are exported so no compiler can inline them away.  Provenance:
 * tests/benchmark/WORKLOAD_PROVENANCE.md. */

extern const unsigned int gzip_crc32_table[256];

unsigned int
gzip_crc32_update_no_xor(unsigned int crc, const unsigned char *buf,
                         unsigned long len)
{
  unsigned long n;

  for (n = 0; n < len; n++)
    crc = gzip_crc32_table[(crc ^ buf[n]) & 0xffU] ^ (crc >> 8);

  return crc;
}
