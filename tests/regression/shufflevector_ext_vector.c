/* `__builtin_shufflevector` on Clang ext vectors (lccc-only: GCC has no
 * ext_vector_type). As in Clang, any index count is valid there, so three
 * indices on a float4 give a float3, and padded 3-lane operands (16-byte
 * storage) are indexed by lane, not by storage slot. The expected values are
 * Clang 23.1.4's output for this program. */
#include <stdio.h>
#include <stdlib.h>

typedef float float4 __attribute__((ext_vector_type(4)));
typedef float float3 __attribute__((ext_vector_type(3)));
typedef float float2 __attribute__((ext_vector_type(2)));
typedef int int3 __attribute__((ext_vector_type(3)));
typedef int int8 __attribute__((ext_vector_type(8)));

__attribute__((noinline)) static float3 f3(float4 a) { return __builtin_shufflevector(a, a, 3, 1, 0); }
__attribute__((noinline)) static float2 f2(float3 a, float3 b) { return __builtin_shufflevector(a, b, 5, 0); }

static int failures;
static void check(const char *what, double got, double want)
{
    if (got != want) {
        printf("FAIL %s: got %g want %g\n", what, got, want);
        ++failures;
    }
}

int main(void)
{
    _Static_assert(sizeof(__typeof__(f3((float4){0}))) == 16, "float3 storage");
    for (volatile int r = 0; r < 2; ++r) {
        float4 a = {1.5f, -2.f, 3.f + r, 4.f};
        float3 t = f3(a);
        float2 u = f2(t, t.zyx);
        int3 i3 = {7, -8, 9 + r};
        int8 w = __builtin_shufflevector(i3, i3, 0, 1, 2, 3, 4, 5, 0, 1);
        int3 n3 = __builtin_shufflevector(i3, i3, 2, 2, 5);
        check("t.x", t.x, 4); check("t.y", t.y, -2); check("t.z", t.z, 1.5);
        check("u.x", u.x, 4); check("u.y", u.y, 4);
        const int want_w[8] = {7, -8, 9 + r, 7, -8, 9 + r, 7, -8};
        for (int i = 0; i < 8; ++i)
            check("w", w[i], want_w[i]);
        check("n3.x", n3.x, 9 + r); check("n3.y", n3.y, 9 + r); check("n3.z", n3.z, 9 + r);
    }
    if (failures)
        abort();
    puts("shufflevector ext vector OK");
    return 0;
}
