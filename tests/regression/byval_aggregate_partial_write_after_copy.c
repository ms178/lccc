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

/* `shape_three` reads `tmp` first, writes `tmp` after its last read so `D`
 * has no remaining reads, and reads `a` only after the write (`D_read ->
 * D_write -> S_use`). Coalescing must still be refused so the write to `tmp`
 * never clobbers `a`, regardless of whether `D` or `S` is read first. */
static long long __attribute__((noinline)) shape_three(int flag)
{
    Big a;
    init(&a, 11, 22, 30, 40, 50, 60);

    long long acc = 0;
    {
        Big tmp = a;
        acc += tmp.h.a * 100 + tmp.tail[0];
        if (flag) {
            tmp.h.a = 999;
        }
        tmp.tail[3] = 777;
    }
    return acc * 10000LL + a.h.a * 100 + a.tail[3];
}

/* `shape_four` runs a copy + write to `D` inside a loop whose header or post-
 * loop observer reads `S`: a loop backedge from a `D_write` block back to `S`
 * must likewise block coalescing. */
static long long __attribute__((noinline)) shape_four(int n)
{
    Big a;
    init(&a, 3, 4, 5, 6, 7, 8);
    long long sum = 0;
    for (int i = 0; i < n; i++) {
        Big tmp = a;
        sum += tmp.h.a + a.tail[1];
        tmp.h.a += 100 + i;
    }
    return sum * 100 + a.h.a;
}

/* `shape_five` reads `S` only at the loop top, then copies `tmp = a` and
 * writes `tmp.h.a = 999` with no same-iteration read of `a` afterwards.
 * Only the loop backedge exposes the stale `a.h.a` on later iterations. */
static long long __attribute__((noinline)) shape_five(int n)
{
    Big a;
    init(&a, 3, 4, 5, 6, 7, 8);
    long long sum = 0;
    for (int i = 0; i < n; i++) {
        sum += a.h.a * 10 + a.tail[0];
        if (i == 1) {
            Big tmp = a;
            tmp.h.a = 999;
            sum += tmp.h.a;
        }
    }
    return sum * 100 + a.h.a;
}

int main(void)
{
    long long r1 = shape_one();
    long long r2 = shape_two();
    long long r3a = shape_three(0);
    long long r3b = shape_three(1);
    long long r4 = shape_four(3);
    long long r5 = shape_five(3);
    printf("r1=%lld r2=%lld r3a=%lld r3b=%lld r4=%lld r5=%lld\n", r1, r2, r3a, r3b, r4, r5);
    return (r1 == 11022077
            && r2 == 505101202707LL
            && r3a == 11301160LL
            && r3b == 11301160LL
            && r4 == 2703
            && r5 == 110403)
        ? 0
        : 1;
}
