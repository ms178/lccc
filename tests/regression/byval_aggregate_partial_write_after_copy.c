/* The copy's destination is written *after* the copy, and only partly.
 *
 *   Big tmp = a;      // 48-byte whole-object copy: the only definition of .tail
 *   tmp.h = h;        // redefines the first 16 bytes
 *   use(tmp.tail);    // still needs what the copy put there
 *
 * Coalescing the copy renames `tmp` onto `a`, so the partial write lands in
 * `a`'s storage.  That is sound only because `a` is dead at the copy — and it
 * must remain sound for the *bytes the copy wrote*, i.e. the rename may not turn
 * the 48-byte copy into a 16-byte one or lose the untouched tail.
 *
 * `shape_two` repeats the shape with the source still live afterwards, which
 * must block the rename entirely and keep the copy; its result mixes the
 * post-copy contents of both objects, so a wrong decision in either direction
 * shows up in the value.
 *
 * `memset` is deliberately avoided: it addresses `a` through a call, which makes
 * the object escape and takes the shape out of every scalar-replacement form. */
#include <stdio.h>

typedef struct {
    long long a, b;
} H;

typedef struct {
    H h;
    long long tail[4];
} Big;

static void init(Big *p, long long ha, long long hb, long long t0, long long t1, long long t2,
                 long long t3)
{
    p->h.a = ha;
    p->h.b = hb;
    p->tail[0] = t0;
    p->tail[1] = t1;
    p->tail[2] = t2;
    p->tail[3] = t3;
}

static long long __attribute__((noinline)) shape_one(void)
{
    Big a;
    init(&a, 1, 2, 11, 22, 33, 44);

    Big tmp = a;
    H h = {7, 8};
    tmp.h = h;
    return tmp.tail[0] * 1000000 + tmp.tail[1] * 1000 + tmp.tail[2] + tmp.tail[3];
}

static long long __attribute__((noinline)) shape_two(void)
{
    Big a;
    init(&a, 5, 6, 101, 202, 303, 404);

    Big tmp = a;
    H h = {9, 10};
    tmp.h = h;

    /* `a` is read after the copy, so the copy has to stay. */
    long long live = a.h.a * 7 + a.h.b * 11 + a.tail[3];
    return live * 1000000000LL
         + tmp.tail[0] * 1000000 + tmp.tail[1] * 1000 + tmp.tail[2] + tmp.tail[3];
}

int main(void)
{
    long long r1 = shape_one();
    long long r2 = shape_two();
    printf("r1=%lld r2=%lld\n", r1, r2);
    return (r1 == 11022077 && r2 == 505101202707LL) ? 0 : 1;
}
