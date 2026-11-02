__attribute__((noinline)) unsigned pc32(unsigned x) {
    return __builtin_popcount(x);
}
__attribute__((noinline)) unsigned pc64(unsigned long long x) {
    return __builtin_popcountll(x);
}
__attribute__((noinline)) unsigned pcs32(int x) {
    return __builtin_popcount(x);
}
__attribute__((noinline)) unsigned pcs64(long long x) {
    return __builtin_popcountll(x);
}
__attribute__((noinline)) unsigned long long pressure(
    unsigned long long a, unsigned long long b, unsigned long long c,
    unsigned long long d, unsigned long long e, unsigned long long f,
    unsigned long long g, unsigned long long h, unsigned long long i,
    unsigned long long j) {
    return __builtin_popcountll(a) + b + 3*c + 5*d + 7*e + 11*f + 13*g + 17*h + 19*i + 23*j;
}
__attribute__((noinline)) unsigned pcu8(unsigned char x) { return __builtin_popcount(x); }
__attribute__((noinline)) unsigned pcs8(signed char x) { return __builtin_popcount(x); }
__attribute__((noinline)) unsigned pcu16(unsigned short x) { return __builtin_popcount(x); }
__attribute__((noinline)) unsigned pcs16(short x) { return __builtin_popcount(x); }
__attribute__((noinline)) unsigned pcl(unsigned long x) { return __builtin_popcountl(x); }
