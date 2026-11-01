/* Byte-copy loops that loop_idiom turns into memcpy behind a nonzero guard
 * (disjoint roots, nullable pointer parameters), plus the self-loop shapes
 * whose trip count is not the compared bound.
 *
 * Zero trips: the guard's false edge is the zero-trip exit, so the live-out
 * index and bump pointers must keep their preheader values and null
 * pointers must never reach memcpy. Nonzero trips: the live-outs are the
 * fast path's `bound` and `init + bound`.
 *
 * `do { ... } while (i++ < n)` copies n + 1 bytes (the test reads the old
 * index after the body ran), so it must not become memcpy(d, s, n); it did
 * at -O2 (one byte short). `if (i < n) break;` at the top iterates while the
 * test is false (nothing is copied for n != 0); it became memcpy(d, s, n).
 *
 * Compared against GCC by the regression corpus. */
#include <stdio.h>
#include <string.h>

#define N 64

__attribute__((noinline)) static unsigned long
copy_index(unsigned char *restrict d, const unsigned char *restrict s, unsigned long n)
{
    unsigned long i;
    for (i = 0; i < n; i++)
        d[i] = s[i];
    return i;
}

__attribute__((noinline)) static unsigned char *
copy_bump(unsigned char *restrict d, const unsigned char *restrict s, unsigned long n)
{
    for (unsigned long i = 0; i < n; i++)
        *d++ = *s++;
    return d;
}

__attribute__((noinline)) static const unsigned char *
copy_bump_src(unsigned char *restrict d, const unsigned char *restrict s, unsigned n)
{
    for (unsigned i = 0; i < n; i++)
        *d++ = *s++;
    return s;
}

/* Local destination: disjoint by freshness, nullable source. */
__attribute__((noinline)) static unsigned
copy_to_local(const unsigned char *s, unsigned n)
{
    unsigned char buf[N];
    unsigned i;
    if (n > N)
        n = N;
    for (i = 0; i < n; i++)
        buf[i] = s[i];
    unsigned h = i * 131u;
    for (unsigned k = 0; k < i; k++)
        h = h * 31u + buf[k];
    return h;
}

/* The compared index is the one the body already used: n + 1 copies. */
__attribute__((noinline)) static unsigned long
copy_do_post(unsigned char *restrict d, const unsigned char *restrict s, unsigned long n)
{
    unsigned long i = 0;
    do {
        d[i] = s[i];
    } while (i++ < n);
    return i;
}

__attribute__((noinline)) static unsigned char *
copy_do_post_bump(unsigned char *restrict d, const unsigned char *restrict s, unsigned n)
{
    unsigned i = 0;
    do {
        *d++ = *s++;
    } while (i++ < n);
    return d;
}

/* Same shapes with no live-out: these reach the copy rewrite. */
__attribute__((noinline)) static void
copy_do_post_void(unsigned char *restrict d, const unsigned char *restrict s, unsigned long n)
{
    unsigned long i = 0;
    do {
        d[i] = s[i];
    } while (i++ < n);
}

__attribute__((noinline)) static void
copy_do_post_bump_void(unsigned char *restrict d, const unsigned char *restrict s, unsigned n)
{
    unsigned i = 0;
    do {
        *d++ = *s++;
    } while (i++ < n);
}

__attribute__((noinline)) static void
copy_break_on_test(unsigned char *restrict d, const unsigned char *restrict s, unsigned long n)
{
    unsigned long i = 0;
    for (;;) {
        if (i < n)
            break;
        d[i] = s[i];
        i++;
    }
}

static unsigned sum(const unsigned char *p, unsigned long n)
{
    unsigned h = 0;
    for (unsigned long k = 0; k < n; k++)
        h = h * 33u + p[k];
    return h;
}

int main(void)
{
    static unsigned char src[N + 2], dst[N + 2];
    for (unsigned k = 0; k < N + 2; k++)
        src[k] = (unsigned char)(k * 7u + 3u);
    unsigned acc = 0;

    /* Zero trips with null pointers: no memcpy, preheader live-outs. */
    acc = acc * 3u + (unsigned)copy_index(NULL, NULL, 0);
    acc = acc * 3u + (copy_bump(NULL, NULL, 0) == NULL);
    acc = acc * 3u + (copy_bump_src(NULL, NULL, 0) == NULL);
    acc = acc * 3u + copy_to_local(NULL, 0);
    acc = acc * 3u + (copy_bump(dst + 5, NULL, 0) == dst + 5);
    acc = acc * 3u + (copy_bump_src(NULL, src + 9, 0) == src + 9);

    for (unsigned long n = 0; n <= N; n += (n < 4 ? 1 : 15)) {
        memset(dst, 0, sizeof dst);
        acc = acc * 3u + (unsigned)copy_index(dst, src, n) + sum(dst, N + 2);
        memset(dst, 0, sizeof dst);
        acc = acc * 3u + (unsigned)(copy_bump(dst, src, n) - dst) + sum(dst, N + 2);
        memset(dst, 0, sizeof dst);
        acc = acc * 3u + (unsigned)(copy_bump_src(dst, src + 1, (unsigned)n) - src)
              + sum(dst, N + 2);
        acc = acc * 3u + copy_to_local(src, (unsigned)n);
        if (n < N) {
            memset(dst, 0, sizeof dst);
            acc = acc * 3u + (unsigned)copy_do_post(dst, src, n) + sum(dst, N + 2);
            memset(dst, 0, sizeof dst);
            acc = acc * 3u + (unsigned)(copy_do_post_bump(dst, src, (unsigned)n) - dst)
                  + sum(dst, N + 2);
            memset(dst, 0, sizeof dst);
            copy_do_post_void(dst, src, n);
            acc = acc * 3u + sum(dst, N + 2);
            memset(dst, 0, sizeof dst);
            copy_do_post_bump_void(dst, src, (unsigned)n);
            acc = acc * 3u + sum(dst, N + 2);
        }
        if (n != 0) {
            memset(dst, 0, sizeof dst);
            copy_break_on_test(dst, src, n);
            acc = acc * 3u + sum(dst, N + 2);
        }
    }
    printf("%08x\n", acc);
    return 0;
}
