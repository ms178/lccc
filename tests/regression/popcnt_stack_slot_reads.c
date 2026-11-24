/* A stack slot read after a `popcnt` is the slot the instruction names.
 *
 * The peephole pass that deletes stores to stack slots nothing reads keeps
 * track of where `%rsp` stands, and it took every mnemonic beginning with
 * `pop` for a pop: after `popcntq %rax, %rdi` it believed `%rsp` stood 8
 * bytes higher, attributed each later `N(%rsp)` to the slot at `N + 8`, and
 * so found no reader for the slots that really were read. Their stores were
 * deleted and the function read uninitialised stack: wrong results at
 * `-O1`, `-O2`, `-O3` and `-Os` for any function without a frame pointer
 * that ran a `popcnt` and then reloaded a spilled value.
 *
 * `mix` keeps twenty values live across the `popcnt`, more than there are
 * registers, so some of them are reloaded after it; `kernel` does the same
 * with the instruction reached through the SSE4.2 intrinsic. The program
 * prints the same with and without POPCNT in the CPU. */
#include <nmmintrin.h>
#include <stdio.h>

__attribute__((noinline, target("popcnt"))) static unsigned long mix(const unsigned long *p) {
    unsigned long a0 = p[0], a1 = p[1], a2 = p[2], a3 = p[3], a4 = p[4];
    unsigned long a5 = p[5], a6 = p[6], a7 = p[7], a8 = p[8], a9 = p[9];
    unsigned long b0 = p[10], b1 = p[11], b2 = p[12], b3 = p[13], b4 = p[14];
    unsigned long b5 = p[15], b6 = p[16], b7 = p[17], b8 = p[18], b9 = p[19];
    unsigned long c = (unsigned long)__builtin_popcountl(p[20]);
    return c + a0 * 3 + a1 * 5 + a2 * 7 + a3 * 11 + a4 * 13 + a5 * 17 + a6 * 19 + a7 * 23 + a8 * 29 +
           a9 * 31 + b0 * 37 + b1 * 41 + b2 * 43 + b3 * 47 + b4 * 53 + b5 * 59 + b6 * 61 + b7 * 67 +
           b8 * 71 + b9 * 73;
}

/* The same sum without the instruction, for a CPU that lacks it. */
__attribute__((noinline)) static unsigned long mix_portable(const unsigned long *p) {
    static const unsigned long k[20] = {3,  5,  7,  11, 13, 17, 19, 23, 29, 31,
                                        37, 41, 43, 47, 53, 59, 61, 67, 71, 73};
    unsigned long sum = 0, bits = p[20];
    for (; bits; bits &= bits - 1)
        sum++;
    for (int i = 0; i < 20; i++)
        sum += p[i] * k[i];
    return sum;
}

__attribute__((target("sse4.1,popcnt,ssse3"))) static int sse41_popcnt(const int *v) {
    __m128i x = _mm_loadu_si128((const __m128i *)v);
    x = _mm_mullo_epi32(x, x);
    return _mm_popcnt_u32((unsigned)_mm_extract_epi32(x, 3));
}

/* `_mm_cvtsi128_si32`'s result waits in a stack slot while the inlined
 * `sse41_popcnt` runs, and is read after its `popcnt`. */
__attribute__((noinline, target("sse4.1,popcnt,ssse3"))) static int kernel(const int *v) {
    __m128i x = _mm_loadu_si128((const __m128i *)v);
    x = _mm_abs_epi32(_mm_sub_epi32(_mm_setzero_si128(), x));
    return _mm_cvtsi128_si32(x) + sse41_popcnt(v);
}

int main(void) {
    static const int v[8] = {1, -2, 3, -4, 5, -6, 7, -8};
    unsigned long w[21];
    for (int i = 0; i < 21; i++)
        w[i] = 0x9e3779b97f4a7c15ul * (unsigned long)(i + 1);
    __builtin_cpu_init();
    int has = __builtin_cpu_supports("popcnt") && __builtin_cpu_supports("sse4.1") &&
              __builtin_cpu_supports("ssse3");
    printf("mix %lu\n", has ? mix(w) : mix_portable(w));
    printf("portable %lu\n", mix_portable(w));
    printf("kernel %d\n", has ? kernel(v) : 2);
    return 0;
}
