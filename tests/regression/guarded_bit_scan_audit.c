/* Guard polarity, header effects and zero paths of shift/count idioms. */
static volatile unsigned visits;
__attribute__((noinline)) unsigned trailing_zeros(unsigned x) {
    if (!x) return 100;
    unsigned r = 1;
    while (!(x & 1)) { x >>= 1; ++r; }
    return r;
}
__attribute__((noinline)) unsigned trailing_ones(unsigned x) {
    if (!x) return 0;
    unsigned r = 0;
    while ((x & 1) == 1) { x >>= 1; ++r; }
    return r;
}
__attribute__((noinline)) unsigned with_header_effect(unsigned x) {
    if (!x) return 100;
    unsigned r = 1;
    while ((++visits, !(x & 1))) { x >>= 1; ++r; }
    return r;
}
__attribute__((noinline)) unsigned long wide_scan(unsigned long x) {
    if (!x) return 100;
    unsigned long r = 5;
    while (!(x & 1)) { x >>= 1; ++r; }
    return r;
}
int main(void) {
    if (trailing_zeros(0) != 100 || wide_scan(0) != 100) return 1;
    for (unsigned bits = 0; bits < 32; ++bits) {
        unsigned x = 1u << bits;
        if (trailing_zeros(x) != bits + 1) return 2;
        visits = 0;
        if (with_header_effect(x) != bits + 1 || visits != bits + 1) return 3;
        unsigned ones = x - 1;
        if (trailing_ones(ones) != bits) return 4;
    }
    if (trailing_ones(~0u) != 32) return 5;
    for (unsigned bits = 0; bits < sizeof(unsigned long) * 8; ++bits)
        if (wide_scan(1ul << bits) != bits + 5) return 6;
    return 0;
}
