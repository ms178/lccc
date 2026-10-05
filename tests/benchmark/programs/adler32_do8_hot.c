/* Distilled zlib-ng Adler-32 DO8 shape — Godbolt inner-loop kernel. */
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
  unsigned i;
  for (i = 0; i < 64; i++)
    buf[i] = (unsigned char)(i + 1);
  return (int)(adler32_len(1, buf, 64, 0) & 255u);
}
