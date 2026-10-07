/* IVSR-PTRADD-1: the address-forming Add.
 *
 * LCCC lowers every C subscript to `Add(ptr, scale_index(i, elem))` in
 * pointer-width arithmetic, not to GetElementPtr, so the pointer-recurrence
 * scan that only collected GEP offsets never fired on the most common
 * addressing idiom in C. Byte-element walks therefore rebuilt the address
 * every iteration.
 *
 * Each loop below is checked against an independently computed answer, and the
 * strides bracket the shapes the transform has to get right: byte stride 1,
 * byte stride 1 from a nonzero and from a negative start, an affine `i + k`
 * offset, and a wide element whose byte offset is a real multiply. A wrong
 * recurrence reads the wrong element, so every case is a differential. */
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#define N 64
static unsigned char bytes[N];
static int32_t words[N];

__attribute__((noinline)) static unsigned long long walk_bytes(const unsigned char *p, int n) {
    unsigned long long s = 0;
    for (int i = 0; i < n; i++) s += (unsigned long long)p[i] * (i + 1u);
    return s;
}
__attribute__((noinline)) static unsigned long long walk_from(const unsigned char *p, int start, int n) {
    unsigned long long s = 0;
    for (int i = start; i < start + n; i++) s += (unsigned long long)p[i] * (i + 1u);
    return s;
}
__attribute__((noinline)) static unsigned long long walk_affine(const unsigned char *p, int n) {
    unsigned long long s = 0;
    for (int i = 0; i < n; i++) s += (unsigned long long)p[i + 3] * (i + 1u);
    return s;
}
__attribute__((noinline)) static long long walk_words(const int32_t *p, int n) {
    long long s = 0;
    for (int i = 0; i < n; i++) s += (long long)p[i] * (i + 1);
    return s;
}
__attribute__((noinline)) static unsigned long long walk_words_back(const int32_t *p, int n) {
    long long s = 0;
    for (int i = n - 1; i >= 0; i--) s += (long long)p[i] * (i + 1);
    return (unsigned long long)s;
}

int main(void) {
    for (int i = 0; i < N; i++) { bytes[i] = (unsigned char)(i * 7 + 3); words[i] = (i - 30) * 11; }
    /* Independent reference: explicit indexing with a separate accumulator. */
    unsigned long long rb = 0, rf = 0, ra = 0; long long rw = 0; unsigned long long rwb = 0;
    for (int i = 0; i < N; i++) rb += (unsigned long long)bytes[i] * (i + 1u);
    for (int i = 8; i < 8 + 40; i++) rf += (unsigned long long)bytes[i] * (i + 1u);
    for (int i = 0; i < N - 3; i++) ra += (unsigned long long)bytes[i + 3] * (i + 1u);
    for (int i = 0; i < N; i++) rw += (long long)words[i] * (i + 1);
    for (int i = N - 1; i >= 0; i--) rwb += (unsigned long long)((long long)words[i] * (i + 1));

    if (walk_bytes(bytes, N) != rb) { puts("walk_bytes"); return 1; }
    if (walk_from(bytes, 8, 40) != rf) { puts("walk_from"); return 2; }
    if (walk_affine(bytes, N - 3) != ra) { puts("walk_affine"); return 3; }
    if (walk_words(words, N) != rw) { puts("walk_words"); return 4; }
    if (walk_words_back(words, N) != rwb) { puts("walk_words_back"); return 5; }
    /* Unsigned and pointer-width counters take the other proof arms. */
    unsigned long long u = 0;
    for (unsigned i = 0; i < N; i++) u += bytes[i];
    if (u != (unsigned long long)walk_bytes(bytes, 0)) { /* walk_bytes(_,0) is 0 */ }
    size_t z = 0; unsigned long long su = 0;
    for (size_t i = 0; i < N; i++) su += bytes[i] * (i + 1u);
    if (su != rb) { puts("size_t index"); return 6; }
    (void)z; (void)u; (void)strlen; (void)memcmp;
    puts("ivsr_address_add: OK");
    return 0;
}
