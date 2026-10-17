/*
 * aggregate_copy_forward: a forwarded temporary whose SOURCE is a path into
 * another forwarded temporary.
 *
 *     tmp1 = memcpy(obj)          -- candidate, copy deleted
 *     tmp2 = memcpy(tmp1 + 32)    -- candidate, source is a PATH into tmp1
 *     ... read tmp2 ...           -- must become obj + 32 + <path>
 *
 * The pass resolved chains only when a candidate's source was exactly
 * another candidate root; here the reads of `tmp2` were redirected to
 * `tmp1 + 32` while `tmp1`'s defining copy had been removed, so `p.hi`
 * was read from uninitialized stack at -O2 (0 on x86-64, garbage on
 * i686). The same hole was the i686 ext_vector_type -O2 failure
 * (ABI-EXTVEC-I386-O2-1): each 32-byte argument travels as two 16-byte
 * XMM images copied from `image` and `image + 16`.
 *
 * `split` is inlined (it is static and tiny), which is what produces the
 * copy chain; `mk` is noinline so the object is opaque at -O2 and nothing
 * folds to constants before the pass runs.
 */
#include <stdio.h>

struct In { long a[4]; };
struct Out { struct In x, y; };
struct Pair { struct In lo, hi; };

static long sum(struct In v) {
    long s = 0;
    for (int i = 0; i < 4; i++)
        s += v.a[i] * (i + 1);
    return s;
}

static struct Pair split(struct Out o) {
    struct Pair p;
    p.lo = o.x;
    p.hi = o.y;
    return p;
}

static struct Pair swap(struct Pair p) {
    struct Pair q;
    q.lo = p.hi;
    q.hi = p.lo;
    return q;
}

__attribute__((noinline)) struct Out mk(long k) {
    struct Out o;
    for (int i = 0; i < 4; i++) {
        o.x.a[i] = k + i;
        o.y.a[i] = k * 10 + i;
    }
    return o;
}

int main(void) {
    struct Out o = mk(3);
    struct Out t = o;          /* tmp1 = copy(obj) */
    struct In h = t.y;         /* tmp2 = copy(tmp1 + 32) */
    struct Pair p = split(o);  /* inlined: sub-object copies out of a copy */
    struct Pair q = swap(split(t)); /* three hops: obj -> t -> split -> swap */
    printf("%ld %ld %ld %ld\n", sum(h), sum(t.x), sum(p.hi), sum(p.lo));
    printf("%ld %ld\n", sum(q.lo), sum(q.hi));
    printf("%ld %ld\n", q.lo.a[3] + p.hi.a[0], h.a[2] - t.y.a[2]);
    return 0;
}
