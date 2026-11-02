/* Distilled zlib-ng Adler-32 DO8 shape — Godbolt inner-loop kernel.
 *
 * Exit 0 on the known answer; a non-zero checksum used as the process
 * status is what turned these files into a red regression-corpus gate.
 */
#include <stdio.h>

unsigned adler32_len(unsigned sum1, const unsigned char *buf, unsigned long len, unsigned sum2) {
  while (len >= 8) {
    sum1 += buf[0]; sum2 += sum1;
    sum1 += buf[1]; sum2 += sum1;
    sum1 += buf[2]; sum2 += sum1;
    sum1 += buf[3]; sum2 += sum1;
    sum1 += buf[4]; sum2 += sum1;
    sum1 += buf[5]; sum2 += sum1;
    sum1 += buf[6]; sum2 += sum1;
    sum1 += buf[7]; sum2 += sum1;
    buf += 8; len -= 8;
  }
  while (len) { --len; sum1 += *buf++; sum2 += sum1; }
  sum1 %= 65521u; sum2 %= 65521u;
  return sum1 | (sum2 << 16);
}

int main(void) {
  unsigned char buf[64];
  unsigned i, c;
  for (i = 0; i < 64; i++)
    buf[i] = (unsigned char)(i + 1);
  c = adler32_len(1, buf, 64, 0);
  if (c != 0xb3000821U)
    return 2;
  printf("%08x\n", c);
  return 0;
}
