/* Clamp with asymmetric bounds: dual-compare saturating web. */
void k23_clamp_u8(unsigned char *d, const unsigned char *a, int n) {
    for (int i = 0; i < n; i++) {
        unsigned t = a[i];
        if (t < 16) t = 16;
        if (t > 235) t = 235;
        d[i] = (unsigned char)t;
    }
}
