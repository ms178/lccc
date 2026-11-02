/* Find first set bit by shifting. */
int ffs1(unsigned int x) {
    if (!x) return 0;
    int r = 1;
    while (!(x & 1)) { x >>= 1; r++; }
    return r;
}
