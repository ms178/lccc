/* A forwarded aggregate copy must not read its source after the source has
 * been changed by a plain store.
 *
 *     struct T d = s;      // memcpy %d <- %s (both non-escaping allocas)
 *     s.a += 7000;         // Store into the SOURCE after the copy
 *     ... d.a ...          // must still see the pre-store value
 *
 * sret_copy_elision phase 2 (as merged in #890) logs a store into the source
 * as Event::Write, which never marks the root opaque, and its program-order
 * check only looks at whole-object copies, so `load d.a` was re-rooted at
 * `s.a` and observed the new value.  Same-block, cross-block and
 * loop-carried shapes are covered.
 *
 * GCC 16.2: "300 7300 / 100 7100 / 5 7005 / 11".
 */
#include <stdio.h>

struct T { long a, b, c; };

__attribute__((noinline)) static struct T seed(long k) {
    struct T t = {k, k + 1, k + 2};
    return t;
}

__attribute__((noinline)) static void same_block(long k) {
    struct T s = seed(k);
    struct T d = s;
    s.a += 7000;
    printf("%ld %ld\n", d.a, s.a);
}

__attribute__((noinline)) static void cross_block(long k, int c) {
    struct T s = seed(k);
    struct T d = s;
    if (c)
        s.a += 7000;
    long r = 0;
    if (k > 0)
        r = d.a;
    printf("%ld %ld\n", r, s.a);
}

__attribute__((noinline)) static long loop_carried(int n) {
    struct T s = seed(1);
    long acc = 0;
    for (int i = 0; i < n; i++) {
        struct T d = s;
        s.a = s.a * 2 + i;       /* source rewritten after each copy */
        acc += d.a + d.c - d.b;  /* must read the copy's snapshot */
    }
    return acc;
}

int main(void) {
    same_block(300);
    cross_block(100, 1);
    cross_block(5, 1);
    printf("%ld\n", loop_carried(3));
    return 0;
}
