/* Dead full-width vector stores (`kill_overwritten_vec16_stores`): a
 * 16-byte store overwritten by a later full-width store to the SAME
 * address with no intervening read is deleted, and ordinary DCE then
 * retires its computation — the
 * `_mm_storeu_si128(p, f(a)); _mm_storeu_si128(p, g(a))` shape.
 *
 * The fail-closed shapes guard the other side: an intervening read of
 * the stored bytes, an opaque call between the stores, aliasing
 * destination pointers and disjoint offsets must all keep both stores.
 * Each result is compared against the reference compiler at runtime.
 */
#include <immintrin.h>
#include <stdio.h>

/* First store dead: the add must disappear with it. */
__attribute__((noinline)) void ds_kill(int *p, __m128i a, __m128i b) {
    _mm_storeu_si128((__m128i *)p, _mm_add_epi32(a, b));
    _mm_storeu_si128((__m128i *)p, _mm_sub_epi32(a, b));
}

/* A scalar read between the stores observes the first one. */
__attribute__((noinline)) int ds_read(int *p, __m128i a, __m128i b) {
    _mm_storeu_si128((__m128i *)p, a);
    int r = p[2];
    _mm_storeu_si128((__m128i *)p, b);
    return r;
}

/* The callee reads the buffer between the stores. */
int peek_sum;
__attribute__((noinline)) void peek(int *p) { peek_sum += p[0] + p[3]; }
__attribute__((noinline)) void ds_call(int *p, __m128i a, __m128i b) {
    _mm_storeu_si128((__m128i *)p, a);
    peek(p);
    _mm_storeu_si128((__m128i *)p, b);
}

/* Two pointers that may alias: both stores must execute. */
__attribute__((noinline)) void ds_alias(int *p, int *q, __m128i a,
                                        __m128i b) {
    _mm_storeu_si128((__m128i *)p, a);
    _mm_storeu_si128((__m128i *)q, b);
}

/* Disjoint offsets: neither store is dead. */
__attribute__((noinline)) void ds_offsets(int *p, __m128i a, __m128i b) {
    _mm_storeu_si128((__m128i *)p, a);
    _mm_storeu_si128((__m128i *)(p + 4), b);
}

/* Three stores in a row: only the last survives, transitively. */
__attribute__((noinline)) void ds_chain(int *p, __m128i a, __m128i b) {
    _mm_storeu_si128((__m128i *)p, a);
    _mm_storeu_si128((__m128i *)p, b);
    _mm_storeu_si128((__m128i *)p, _mm_xor_si128(a, b));
}

static void show(const char *tag, const int *p, int n) {
    printf("%s", tag);
    for (int i = 0; i < n; i++) printf(" %d", p[i]);
    printf("\n");
}

int main(void) {
    __m128i a = _mm_set_epi32(40, 30, 20, 10);
    __m128i b = _mm_set_epi32(-4, -3, -2, -1);
    int buf[8];

    ds_kill(buf, a, b);
    show("kill", buf, 4);
    printf("read r=%d\n", ds_read(buf, a, b));
    show("read-buf", buf, 4);
    ds_call(buf, a, b);
    printf("call peek=%d\n", peek_sum);
    show("call-buf", buf, 4);
    ds_alias(buf, buf, a, b);
    show("alias", buf, 4);
    ds_alias(buf, buf + 1, a, b); /* overlapping by 12 bytes */
    show("alias-overlap", buf, 5);
    ds_offsets(buf, a, b);
    show("offsets", buf, 8);
    ds_chain(buf, a, b);
    show("chain", buf, 4);
    return 0;
}
