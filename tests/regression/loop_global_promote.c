/* Loop promotion of unexposed static globals + escaped-alloca LICM.
 *
 * Pins three related pieces of machinery (all compared against the GCC
 * oracle):
 *
 * 1. `loop_memory_promote` tier 2: a `static` global whose address is never
 *    exposed anywhere in the module is promoted out of FNV/CRC-style
 *    `global = f(global, buf[k])` loops even though `buf` is an unknown
 *    caller pointer — an unknown pointer cannot legally point into an
 *    unexposed internal global. The writeback is GUARDED by a did-iterate
 *    flag: the zero-trip path must not store (an unconditional writeback
 *    could revert a concurrent thread's update — classic lost-update
 *    introduced by the compiler).
 *
 * 2. The store no longer needs to sit IN the latch: a rotated-later loop
 *    keeps `load; op; store` in a body block that branches to a separate
 *    increment latch. Soundness contract (mutation-tested during bring-up):
 *    store block DOMINATES the latch, the load PRECEDES the store (block
 *    index or strict dominance — dominators of the latch form a chain),
 *    neither access sits in a nested loop, and the exit writeback value is
 *    chosen by store-dominates-exit, not exit==latch.
 *
 * 3. LICM: loads from an ADDRESS-TAKEN alloca now hoist when every modeled
 *    in-loop store traces to distinct storage (another alloca or a global),
 *    and bounded const-offset frame-slot accesses are speculation-safe like
 *    bounded defined-global accesses (`s.z`/`s.n` re-loaded every byte in
 *    sqlite_vdbe_serial's digest loop because the cell escaped to a decode
 *    call OUTSIDE the loop).
 *
 * The `expose_*` functions deliberately take the digest's address, which
 * must disqualify THAT global from tier 2 (an exposed global may be aliased
 * by any unknown pointer) while leaving the others promotable; the output
 * must stay identical either way.
 */
#include <stdio.h>
#include <string.h>

static unsigned long fnv_digest;      /* promotable: address never taken */
static unsigned long exposed_digest;  /* NOT promotable: address escapes */
static unsigned long rotated_acc;     /* promotable, do-while shape */

__attribute__((noinline))
static void fnv_fold(const unsigned char *z, unsigned n) {
    for (unsigned k = 0; k < n; k++)
        fnv_digest = (fnv_digest ^ z[k]) * 1099511628211UL;
}

__attribute__((noinline))
static unsigned long *expose(void) { return &exposed_digest; }

__attribute__((noinline))
static void exposed_fold(const unsigned char *z, unsigned n) {
    for (unsigned k = 0; k < n; k++)
        exposed_digest = (exposed_digest ^ z[k]) * 1099511628211UL;
}

/* buf aliases the exposed global: the loop must observe its own updates
 * byte by byte. Promotion of `exposed_digest` here would be a miscompile;
 * the exposure scan must have disqualified it. */
__attribute__((noinline))
static unsigned long self_alias_fold(unsigned n) {
    unsigned char *p = (unsigned char *)expose();
    for (unsigned k = 0; k < n && k < sizeof(unsigned long); k++)
        exposed_digest = (exposed_digest ^ p[k]) * 31UL;
    return exposed_digest;
}

/* Bottom-tested: the store's block dominates the latch AND the exit, so
 * the writeback must use the just-stored value (store-dominates-exit). */
__attribute__((noinline))
static void rotated_fold(const unsigned char *z, unsigned n) {
    unsigned k = 0;
    do {
        rotated_acc = rotated_acc * 33UL + z[k];
        k++;
    } while (k < n);
}

/* Nested shape: the inner loop re-reads the global the outer iteration
 * keeps updating. Outer-level promotion must be declined (accesses sit in
 * a nested loop); inner-level promotion is fine and must stay correct. */
__attribute__((noinline))
static unsigned long nested_fold(const unsigned char *z, unsigned n) {
    unsigned long out = 0;
    for (unsigned i = 0; i < n; i++) {
        for (unsigned k = 0; k < (i & 3u); k++)
            fnv_digest = (fnv_digest ^ z[k]) * 16777619UL;
        out += fnv_digest >> (i & 7u);
    }
    return out;
}

/* LICM shape: escaped struct, decode call outside the loop, global store
 * inside it. `cell.z`/`cell.n` must hoist; the result must not change. */
typedef struct { const unsigned char *z; unsigned n; long pad; } Cell;

__attribute__((noinline))
static void decode(Cell *c, const unsigned char *base, unsigned which) {
    c->z = base + (which % 7u);
    c->n = 3u + (which % 11u);
    c->pad = (long)which * 3;
}

__attribute__((noinline))
static unsigned long licm_scan(const unsigned char *base, unsigned rounds) {
    Cell cell;
    unsigned long total = 0;
    for (unsigned r = 0; r < rounds; r++) {
        decode(&cell, base, r);
        for (unsigned k = 0; k < cell.n; k++)
            fnv_digest = (fnv_digest ^ cell.z[k]) * 1099511628211UL;
        total += cell.n + (unsigned long)cell.pad;
    }
    return total;
}

int main(void) {
    static unsigned char buf[256];
    for (unsigned i = 0; i < sizeof(buf); i++)
        buf[i] = (unsigned char)(i * 131 + 17);

    fnv_digest = 1469598103934665603UL;
    exposed_digest = 88172645463325252UL;
    rotated_acc = 5381;

    fnv_fold(buf, 0);              /* zero-trip: writeback must not run */
    unsigned long after_zero_trip = fnv_digest;
    fnv_fold(buf, 1);
    fnv_fold(buf, 200);
    exposed_fold(buf, 64);
    unsigned long self = self_alias_fold(8);
    rotated_fold(buf, 1);          /* single-iteration do-while */
    rotated_fold(buf, 177);
    unsigned long nest = nested_fold(buf, 41);
    unsigned long scan = licm_scan(buf, 57);

    printf("%lu %lu %lu %lu %lu %lu %lu\n", after_zero_trip, fnv_digest,
           exposed_digest, self, rotated_acc, nest, scan);
    return 0;
}
