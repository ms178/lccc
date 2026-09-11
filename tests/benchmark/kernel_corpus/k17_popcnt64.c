/* Population-count reduction over 64-bit lanes. */
long k17_popcnt64(const unsigned long long *a, int n) {
    long s = 0;
    for (int i = 0; i < n; i++) s += __builtin_popcountll(a[i]);
    return s;
}
