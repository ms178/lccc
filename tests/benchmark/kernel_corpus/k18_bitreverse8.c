/* Nibble-reversal LUT over bytes: table load + shuffle-shaped web. */
static const unsigned char k18_nib[16] = {0, 8, 4, 12, 2, 10, 6, 14,
                                           1, 9, 5, 13, 3, 11, 7, 15};
void k18_bitreverse8(unsigned char *d, const unsigned char *a, int n) {
    for (int i = 0; i < n; i++)
        d[i] = (unsigned char)((k18_nib[a[i] & 15] << 4) | k18_nib[a[i] >> 4]);
}
