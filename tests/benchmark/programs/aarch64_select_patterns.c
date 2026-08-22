/* Branchless integer selection corpus distilled from code-generation defects.
   Header-free declarations keep it usable with cross-target Compiler Explorer. */
extern int printf(const char *, ...);

typedef unsigned int u32;
typedef unsigned short u16;

__attribute__((noinline)) u32 conditional_increment(u32 count, u32 condition) {
    return condition ? count + 1u : count;
}

__attribute__((noinline)) u32 narrow_high_constant(u16 value) {
    return value == (u16)-2;
}

__attribute__((noinline)) u32 select_pressure(
    u32 condition, u32 a, u32 b, u32 c, u32 d, u32 e
) {
    u32 x = condition ? a : b;
    u32 y = condition ? c : d;
    u32 z = condition ? e : a;
    return (condition ? y : z) + x + e;
}

int main(void) {
    u32 state = 1;
    u32 count = 0;
    u32 checksum = 0;
    for (u32 i = 0; i < 50000000u; ++i) {
        state = state * 1664525u + 1013904223u;
        count = conditional_increment(count, state & 1u);
        checksum += select_pressure(state & 8u, state, i, count, state >> 3,
                                    checksum);
    }
    checksum ^= narrow_high_constant((u16)-2);
    printf("%u %u\n", count, checksum);
    return 0;
}
