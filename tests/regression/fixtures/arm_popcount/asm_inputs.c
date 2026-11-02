/* User SIMD instructions are allowed; compiler-generated operand transfers
 * remain outside provenance markers and are forbidden in general-regs-only.
 * Observe raw bits (including negative zero), not just floating equality. */
typedef unsigned long u64;
typedef u64 vec2 __attribute__((vector_size(16)));

__attribute__((noinline)) static u64 double_bits(const double *p) {
    u64 result;
    __asm__ volatile("fmov %0, %d1" : "=r"(result) : "w"(*p));
    return result;
}
__attribute__((noinline)) static unsigned float_bits(const float *p) {
    unsigned result;
    __asm__ volatile("fmov %w0, %s1" : "=r"(result) : "w"(*p));
    return result;
}
__attribute__((noinline)) static u64 integer_bits(u64 x) {
    u64 result;
    __asm__ volatile("fmov %0, %d1" : "=r"(result) : "w"(x));
    return result;
}
__attribute__((noinline)) static u64 high_lane(const vec2 *p) {
    u64 result;
    __asm__ volatile("umov %0, %1.d[1]" : "=r"(result) : "w"(*p));
    return result;
}
__attribute__((noinline)) static u64 high_lane_fixed(const vec2 *p, u64 key) {
    register u64 held __asm__("x17") = key;
    u64 result;
    __asm__ volatile("umov %0, %2.d[1]\n\teor %0, %0, %1"
                     : "=&r"(result) : "r"(held), "w"(*p));
    return result;
}
int test(void) {
    const double d[] = {1.25, -2.5, -0.0};
    const u64 db[] = {0x3ff4000000000000UL, 0xc004000000000000UL, 0x8000000000000000UL};
    const float f[] = {1.25f, -2.5f, -0.0f};
    const unsigned fb[] = {0x3fa00000U, 0xc0200000U, 0x80000000U};
    for (int i = 0; i < 3; ++i) {
        if (double_bits(d+i) != db[i]) return 1;
        if (float_bits(f+i) != fb[i]) return 2;
        if (integer_bits(db[i]) != db[i]) return 3;
    }
    const vec2 v = {0x123456789abcdef0UL, 0xfedcba9876543210UL};
    if (high_lane(&v) != 0xfedcba9876543210UL) return 4;
    if (high_lane_fixed(&v, 0xabcdef1234567890UL) !=
        (0xfedcba9876543210UL ^ 0xabcdef1234567890UL)) return 5;
    return 0;
}
