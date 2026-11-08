/* Exercise a vector snapshot across a write reached via integer addresses.
 * This is a C-level integration control, not the mutation-sensitive IR
 * reproducer in vector_temp_promotion's unit tests. */
#include <immintrin.h>
#include <stdint.h>

__attribute__((noinline)) static int probe(unsigned offset)
{
    int a[8] __attribute__((aligned(32))) = {0};
    int b[8] __attribute__((aligned(32))) = {1, 2, 3, 4, 5, 6, 7, 8};
    uintptr_t distance = (uintptr_t)b - (uintptr_t)a;
    uintptr_t address = (uintptr_t)a + distance + offset;
    __m256i snapshot = _mm256_loadu_si256((const __m256i *)b);
    *(int *)address = 100;
    __m256i result = _mm256_add_epi32(snapshot, _mm256_set1_epi32(1));
    return _mm256_extract_epi32(result, 0) == 2 &&
           _mm256_extract_epi32(result, 1) == 3 &&
           b[offset / sizeof(int)] == 100;
}

int main(void)
{
    return !(probe(0) && probe(sizeof(int)));
}
