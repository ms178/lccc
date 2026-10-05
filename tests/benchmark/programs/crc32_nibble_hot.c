/* Distilled GNU gzip crc32 scalar table update — Godbolt inner-loop kernel. */
unsigned gzip_crc32_update(unsigned crc, const unsigned char *buf, unsigned long len) {
  static const unsigned table[16] = {
    0x00000000, 0x1db71064, 0x3b6e20c8, 0x26d930ac,
    0x76dc4190, 0x6b6b51f4, 0x4db26158, 0x5005713c,
    0xedb88320, 0xf00f9344, 0xd6d6a3e8, 0xcb61b38c,
    0x9b64c2b0, 0x86d3d2d4, 0xa00ae278, 0xbdbdf21c
  };
  unsigned long n;
  for (n = 0; n < len; n++)
    crc = table[(crc ^ buf[n]) & 15u] ^ (crc >> 4);
  return crc;
}

int main(void) {
  unsigned char buf[64];
  unsigned i, c = 0;
  for (i = 0; i < 64; i++)
    buf[i] = (unsigned char)i;
  c = gzip_crc32_update(0, buf, 64);
  return (int)(c & 255u);
}
