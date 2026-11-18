/* A 16-byte vector call result copied into a phi web that the map-vecreg
 * allocator homes in an XMM register ACROSS blocks.
 *
 * The web (`out`) joins two definitions: the direct call's `%xmm0` return
 * and the indirect call's result rebuilt through memory. The allocator gave
 * the web a register home (%xmm2 in the measured reproduction), so the join
 * block reads that REGISTER, per `sse_load_arg`'s reg_assignments-first
 * contract. The direct path's `Copy { web <- call_result }` went through
 * `emit_copy_i128_impl`, which knew only slot destinations: it degenerated
 * into a same-slot GPR round trip the peephole then erased, so the join
 * read whatever the callee left in the home register — every byte wrong on
 * the very first direct call (the public_simd_abi BITS=128 failure).
 *
 * Mirrors tests/regression/public_simd_abi/boundary.c's caller shape in one
 * translation unit: nine vector arguments overflow the XMM argument file so
 * the result web cannot ride in an argument register, and the volatile
 * indirect call creates the second web definition that earns the register
 * home. No vector arithmetic on the caller side can recompute (and thereby
 * hide) a wrong copy.
 */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

#define N 16
#define PARAMETERS                                                          \
    int cookie, __m128 a, __m128 b, __m128 middle, __m128 c, __m128 d,      \
        __m128 e, __m128 f, __m128 g, __m128 h, __m128 i, long long tail

__attribute__((noinline)) __m128 boundary(PARAMETERS)
{
    const __m128 values[9] = {a, b, c, d, e, f, g, h, i};
    unsigned char result[N];
    for (int lane = 0; lane < N; ++lane) {
        unsigned v = (unsigned)cookie
            + (unsigned)((unsigned long long)tail >> (8 * (lane % 8)));
        v ^= ((const unsigned char *)&middle)[lane];
        for (int arg = 0; arg < 9; ++arg)
            v = v * 33u + ((const unsigned char *)&values[arg])[(lane + arg * 3) % N];
        result[lane] = (unsigned char)v;
    }
    __m128 out;
    memcpy(&out, result, N);
    return out;
}

int main(void)
{
    unsigned char bytes[9][N], mid[N], expected[N], actual[N];
    __m128 values[9], middle;
    for (int iteration = 0; iteration < 17; ++iteration) {
        for (int arg = 0; arg < 9; ++arg) {
            for (int lane = 0; lane < N; ++lane)
                bytes[arg][lane] =
                    (unsigned char)(iteration * 97 + arg * 43 + lane * 13);
            memcpy(&values[arg], bytes[arg], N);
        }
        for (int lane = 0; lane < N; ++lane)
            mid[lane] = (unsigned char)(lane * 19 + iteration * 7);
        memcpy(&middle, mid, N);
        int cookie = 0x1234567 + iteration;
        long long tail = 0x123456789abcdefLL + iteration;
        for (int lane = 0; lane < N; ++lane) {
            unsigned v = (unsigned)cookie
                + (unsigned)((unsigned long long)tail >> (8 * (lane % 8)));
            v ^= mid[lane];
            for (int arg = 0; arg < 9; ++arg)
                v = v * 33u + bytes[arg][(lane + arg * 3) % N];
            expected[lane] = (unsigned char)v;
        }
        __m128 (*volatile indirect)(PARAMETERS) = boundary;
        for (int mode = 0; mode < 2; ++mode) {
#define ARGS cookie, values[0], values[1], middle, values[2], values[3], \
             values[4], values[5], values[6], values[7], values[8], tail
            __m128 out = mode ? indirect(ARGS) : boundary(ARGS);
#undef ARGS
            memcpy(actual, &out, N);
            if (memcmp(actual, expected, N)) {
                printf("mismatch: iteration=%d indirect=%d\n", iteration, mode);
                for (int byte = 0; byte < N; ++byte)
                    printf("%02x/%02x%c", actual[byte], expected[byte],
                           byte + 1 == N ? '\n' : ' ');
                return 1;
            }
        }
    }
    puts("vec call ret vecreg web: OK");
    return 0;
}
