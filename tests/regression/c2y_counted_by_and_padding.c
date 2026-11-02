/* GCC 16 / C2y counted_by + __builtin_counted_by_ref + object size +
 * __builtin_clear_padding + __builtin_assoc_barrier. */
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

struct PlainFam {
    int hdr;
    int data[];
};

struct UCountFam {
    uint16_t flags;
    size_t len;
    uint32_t items[] __attribute__((counted_by(len)));
};

struct SCountFam {
    int count;
    uint64_t vals[] __attribute__((counted_by(count)));
};

struct PtrCounted {
    size_t n;
    uint32_t *ptr __attribute__((counted_by(n)));
};

_Static_assert(__builtin_counted_by_ref(((struct PlainFam *)0)->data) == (void *)0,
               "plain fam has no counted_by");

struct InnerPad {
    char c;
    int x;
};

struct ComplexPad {
    char a;
    unsigned int bf1 : 3;
    unsigned int : 0;
    unsigned int bf2 : 5;
    struct InnerPad inner;
    short tail;
    int64_t wide;
    char end_byte;
};

__attribute__((noinline)) double assoc_test(double a, double b, double c) {
    return __builtin_assoc_barrier(a + b) + c;
}

int main(void) {
    struct PlainFam *pf = malloc(sizeof(*pf) + 4 * sizeof(int));
    struct UCountFam *uf = calloc(1, sizeof(*uf) + 8 * sizeof(uint32_t));
    struct SCountFam *sf = calloc(1, sizeof(*sf) + 6 * sizeof(uint64_t));
    if (!pf || !uf || !sf) return 1;

    if (__builtin_counted_by_ref(pf->data) != (void *)0) return 2;

    size_t *len_ptr = __builtin_counted_by_ref(uf->items);
    if (len_ptr != &uf->len) return 3;
    *len_ptr = 7;
    if (uf->len != 7) return 4;
    if (__builtin_dynamic_object_size(uf->items, 1) != 7 * sizeof(uint32_t)) return 5;
    if (__builtin_dynamic_object_size(uf->items, 0) != 7 * sizeof(uint32_t)) return 6;

    int *cnt_ptr = __builtin_counted_by_ref(sf->vals);
    if (cnt_ptr != &sf->count) return 7;
    *cnt_ptr = 5;
    if (__builtin_dynamic_object_size(sf->vals, 1) != 5 * sizeof(uint64_t)) return 8;
    *cnt_ptr = -3;
    if (__builtin_dynamic_object_size(sf->vals, 1) != 0) return 9;

    uint32_t backing[9] = {0};
    struct PtrCounted pc = {0, backing};
    size_t *p_cnt = __builtin_counted_by_ref(pc.ptr);
    if (p_cnt != &pc.n) return 19;
    *p_cnt = 9;
    if (__builtin_dynamic_object_size(pc.ptr, 1) != 9 * sizeof(uint32_t)) return 20;

    /* __builtin_object_size subobject & pointer-offset & side-effect checks */
    char buf[32];
    struct {
        int hdr;
        char payload[16];
        int tail;
    } pkt;
    int side = 0;
    size_t bos_se = __builtin_object_size((side++, buf), 0);
    if (side != 0) return 10;
    (void)bos_se;
    if (__builtin_object_size(buf, 0) != 32) return 11;
    if (__builtin_object_size(buf + 10, 0) != 22) return 12;
    if (__builtin_object_size(&buf[10], 0) != 22) return 13;
    if (__builtin_object_size(&pkt.payload, 1) != 16) return 14;
    if (__builtin_object_size(&pkt.payload, 0) != 20) return 15;
    if (__builtin_object_size("hello", 0) != 6) return 16;

    /* __builtin_clear_padding: fill whole struct with 0xAA, assign all real
     * fields, run __builtin_clear_padding, and compare byte-for-byte against a
     * zero-initialized reference struct with identical field values! */
    struct ComplexPad dirty, clean;
    memset(&dirty, 0xAA, sizeof(dirty));
    memset(&clean, 0x00, sizeof(clean));

    dirty.a = clean.a = 0x5A;
    dirty.bf1 = clean.bf1 = 5;
    dirty.bf2 = clean.bf2 = 19;
    dirty.inner.c = clean.inner.c = 0x33;
    dirty.inner.x = clean.inner.x = 0x12345678;
    dirty.tail = clean.tail = 0x1357;
    dirty.wide = clean.wide = 0x0102030405060708LL;
    dirty.end_byte = clean.end_byte = 0x7E;

    __builtin_clear_padding(&dirty);
    if (memcmp(&dirty, &clean, sizeof(dirty)) != 0) {
        return 17;
    }

    double r = assoc_test(1e16, -1e16, 1.0);
    if (r != 1.0) return 18;

    printf("counted_by_and_padding=OK bos=%zu,%zu dsize=%zu\n",
           __builtin_object_size(&pkt.payload, 0),
           __builtin_object_size(&pkt.payload, 1),
           __builtin_dynamic_object_size(uf->items, 1));

    free(pf);
    free(uf);
    free(sf);
    return 0;
}
