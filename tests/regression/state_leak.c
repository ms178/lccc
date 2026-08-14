/* CodegenState per-function sets (vector_values,
 * vector128_values, protected_slot_values, vector_defer_values) were never
 * cleared by reset_for_function(). Value IDs are function-local, so a scalar
 * value in function B whose ID collided with a vector-intrinsic result from
 * function A (compiled earlier) was treated as a vector: its Copy was emitted
 * via the vector path (LEA instead of LOAD) and the value became the address
 * of its own slot.
 * Failing shape (pre-fix): zlib-ng-style adler32 (vector_defer_multidef_slot.c)
 * — vs2 was initialized with the stack address of its slot (wide-loop lanes
 * [0x259b,0x13f,...] instead of the weighted sums).
 * This test compiles a tiny vector-using function FIRST so its SSA value IDs
 * are small, then a scalar loop whose colliding IDs must stay scalar. */
#include <immintrin.h>
#include <stdint.h>

/* compiled first: small function, small SSA value IDs */
static uint32_t vsum(__m128i x) {
    __m128i t = _mm_srli_si128(x, 8);
    return (uint32_t)_mm_cvtsi128_si32(_mm_add_epi32(x, t));
}

/* scalar loop whose value IDs overlap the vector function's ID space */
static uint32_t scalar_sum(uint32_t n) {
    uint32_t s = 0;
    for (uint32_t i = 1; i <= n; i++) s += i;
    return s;
}

int main(void) {
    __m128i v = _mm_setr_epi32(1, 2, 3, 4);
    uint32_t a = vsum(v);
    uint32_t b = scalar_sum(100);
    if (a != 4) return 1;      /* lane0+lane2 = 1+3 (srli 8) */
    if (b != 5050) return 2;   /* pre-fix: stack-address garbage */
    if (a + b != 5054) return 3;
    return 0;
}
