/* Backward copy: negative-stride store chain (memmove shape). */
void k22_copy_backward(unsigned char *d, const unsigned char *a, int n) {
    for (int i = n - 1; i >= 0; i--) d[i] = a[i];
}
