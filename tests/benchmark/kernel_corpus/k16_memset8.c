/* Byte fill: contiguous store chain with loop-carried pointer. */
void k16_memset8(unsigned char *d, unsigned char v, int n) {
    for (int i = 0; i < n; i++) d[i] = v;
}
