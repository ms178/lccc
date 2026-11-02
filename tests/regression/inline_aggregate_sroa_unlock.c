/*
 * Aggregate promotion through inlined helpers, and the escape that must stop it.
 *
 * Three shapes share one table:
 *
 *   read_only()  copies a small struct out of the table into a local and reads
 *                two fields back.  Inlining it removes the last escape of
 *                `encode_symbol`'s local, which is what lets the aggregate be
 *                split into registers; the copy itself is then elided because
 *                the local is only ever read (`elide_readonly_local_memcpys`).
 *
 *   escaped()    copies the same struct, takes the local's address into a
 *                second variable and writes through it.  The address escapes,
 *                so neither the split nor the copy elision may happen: the
 *                writes must land in the local, not in the table.
 *
 *   indirect()   passes `&local` to a static-inline helper and reads it back.
 *                The alloca's address reaches a call that the inliner removes;
 *                after inlining the fields are promoted like any other local.
 *
 * A pointer field (`slot.next`) pins the `Ptr` arm of `promotable_width`.
 *
 * The table is `const`, so a transformation that redirects a write through an
 * elided local into the source would either crash or produce a different
 * checksum: this test is the tripwire for both.
 */
#include <stdio.h>
#include <stdint.h>

typedef struct {
    int32_t lo;
    int32_t hi;
} pair_t;

static const pair_t table[16] = {
    {  1,    2}, {  3,    5}, {  7,   11}, { 13,   17},
    { -1,   -2}, { -3,   -5}, { -7,  -11}, {-13,  -17},
    {100, -200}, {-300,  400}, {  5,   -6}, { -7,    8},
    {  9,   10}, { -11,  -12}, { 13,  -14}, {-15,   16},
};

/* Read-only copy: the local is a pure load source after the copy. */
static inline int32_t read_only(int idx, int32_t k) {
    const pair_t p = table[idx & 15];
    return p.lo * 3 - p.hi * 5 + k;
}

/* Addressed copy: the local escapes and is written through. */
static inline int32_t escaped(int idx) {
    pair_t p = table[idx & 15];
    pair_t *q = &p;
    q->lo += 7;
    q->hi -= 3;
    return (int32_t)((uint32_t)q->lo ^ (uint32_t)q->hi);
}

/* Caller-local aggregate whose address is handed to a static-inline helper. */
static inline void bump(pair_t *p, int32_t by) {
    p->lo += by;
    p->hi -= by;
}

static inline int32_t indirect(int idx) {
    pair_t p = table[idx & 15];
    bump(&p, idx);
    bump(&p, 3);
    return p.lo * 2 + p.hi;
}

/* Struct with a pointer field: exercises Ptr fields in promotable_width. */
typedef struct {
    const pair_t *slot;
    int32_t count;
} view_t;

static inline int64_t sum_view(const view_t *v) {
    int64_t s = 0;
    for (int32_t i = 0; i < v->count; i++) {
        s += (int64_t)v->slot[i].lo * (int64_t)v->slot[i].hi;
    }
    return s;
}

int main(void) {
    int64_t acc = 0;
    for (int idx = 0; idx < 64; idx++) {
        acc += read_only(idx, idx * 7);
        acc += escaped(idx);
        acc += indirect(idx);
    }
    view_t v = { table, 16 };
    acc += sum_view(&v);
    /* Fold the high bits back in so a truncated or widened intermediate
       changes the printed value instead of hiding inside it. */
    acc = (acc ^ (acc >> 17)) * 2654435761u;
    printf("%lld\n", (long long)acc);
    return 0;
}
