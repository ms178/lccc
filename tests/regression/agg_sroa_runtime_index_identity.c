/* aggregate_sroa must treat a runtime-offset pointer into an object as a
 * pointer into THAT object.
 *
 * The pass's derivation map recorded only constant-offset GEPs, so `&s.v[k]`
 * resolved to itself, an apparently unrelated object.  Every identity question
 * Transform 1 (load forwarding through a single-copy alloca) asks then had the
 * wrong answer:
 *
 *   src_store   memcpy d <- s; s.v[k] = X; load d.v[i]
 *               the store did not dirty the source; the load read X.
 *   dst_store   memcpy d <- s; d.v[k] = X; load d.v[i]
 *               d did not look stored-to; the load was forwarded to s.
 *   dst_read    memcpy d <- s; load d.v[k]; load d.v[0]
 *               the runtime read was not counted, every *counted* read was
 *               forwarded, and the now "dead" copy was deleted under it.
 *   escape      memcpy d <- s; clobber(&s.v[k]); load d.v[i]
 *               s did not escape through the call, so as a "private" alloca
 *               it was assumed untouchable by it.
 *   loop        the src_store shape carried around a loop.
 *
 * GCC 16.2 -O2: "11321 4323 / 4721 4327 / 3 4 / 321 4321 / 212415".
 */
#include <stdio.h>

struct T { long v[4]; };

__attribute__((noinline)) long src_store(const struct T *in, int k) {
    struct T s = *in;
    struct T d = s;
    s.v[k] = 7000;
    return d.v[0] + d.v[1] * 10 + d.v[2] * 100 + d.v[3] * 1000 + s.v[1];
}

__attribute__((noinline)) long dst_store(const struct T *in, int k) {
    struct T s = *in;
    struct T d = s;
    d.v[k] = 7;
    return d.v[0] + d.v[1] * 10 + d.v[2] * 100 + d.v[3] * 1000 + s.v[0] - 1;
}

__attribute__((noinline)) long dst_read(const struct T *in, int k) {
    struct T s = *in;
    struct T d = s;
    long t = d.v[k];
    return t + d.v[0] - 1;
}

__attribute__((noinline)) void clobber(long *p) { *p = 9000; }

__attribute__((noinline)) long escape(const struct T *in, int k) {
    struct T s = *in;
    struct T d = s;
    clobber(&s.v[k]);
    return d.v[0] + d.v[1] * 10 + d.v[2] * 100 + (s.v[3] != 9000) * 4000;
}

__attribute__((noinline)) long loop(const struct T *in, int n) {
    struct T s = *in;
    long acc = 0;
    for (int i = 0; i < n; i++) {
        struct T d = s;
        s.v[i & 3] += 5;
        acc = acc * 7 + d.v[0] + d.v[1] + d.v[2] + d.v[3];
    }
    return acc;
}

int main(void) {
    struct T a = {{1, 2, 3, 4}};
    printf("%ld %ld\n", src_store(&a, 1), src_store(&a, 3));
    printf("%ld %ld\n", dst_store(&a, 2), dst_store(&a, 0));
    printf("%ld %ld\n", dst_read(&a, 2), dst_read(&a, 3));
    printf("%ld %ld\n", escape(&a, 3), escape(&a, 0));
    printf("%ld\n", loop(&a, 6));
    return 0;
}
