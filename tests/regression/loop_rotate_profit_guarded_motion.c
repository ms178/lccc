/* Loop rotation where it pays (RotatePolicy::Profit, the default at -O2).
 *
 * The pass turns `while (c) body` into `if (c) do body while (c)` for the
 * loops out of which something then moves that must not run unless the body
 * does: a load, a division, a `*p += x` or `a[i] = f(a[i-1])` recurrence
 * kept in a register. This file pins the one thing that makes that legal:
 * NOTHING THAT MOVED MAY RUN WHEN THE LOOP DOES NOT.
 *
 * Every function is called first with a trip count of zero and operands
 * that fault when touched (a PROT_NONE page for loads and stores, a
 * read-only page for stores, a zero divisor and LONG_MIN / -1 for
 * divisions), then with ordinary operands, where the result is compared
 * against a reference that the optimizer cannot rewrite (every access is
 * `volatile`) and against GCC by the harness. The aliasing rows call the
 * same functions with operands that overlap in every way the C rules allow
 * for these types.
 *
 * No .env file: the test runs the default pipeline. With
 * CCC_NO_LOOP_ROTATE=1 it passes as well (nothing moves), which is the
 * point: the two must be indistinguishable from the outside.
 */
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>

#define NOINLINE __attribute__((noinline))

struct counter {
    long pad;
    long count;
    double mass;
};

/* ---- the shapes, as a compiler meets them ------------------------------ */

/* `s->count += f(i)`: the promoter keeps the field in a register behind the
 * guard and writes it back on the exit edge. */
NOINLINE void acc_field(struct counter *s, long n) {
    for (long i = 0; i < n; i++)
        s->count += i * 3;
}

/* The same through a bare pointer, with a value that depends on a second
 * array which may be the accumulator's own storage. */
NOINLINE void acc_array(long *p, const long *a, long n) {
    for (long i = 0; i < n; i++)
        *p += a[i] ^ 5;
}

/* A loop-invariant load in the body. */
NOINLINE long inv_load(const long *p, const long *a, long n) {
    long s = 0;
    for (long i = 0; i < n; i++)
        s += a[i] * *p;
    return s;
}

/* A loop-invariant division in the body. */
NOINLINE long inv_div(long a, long b, long n) {
    long s = 0;
    for (long i = 0; i < n; i++)
        s += a / b + i;
    return s;
}

/* A loop-invariant remainder, unsigned. */
NOINLINE unsigned long inv_rem(unsigned long a, unsigned long b, long n) {
    unsigned long s = 0;
    for (long i = 0; i < n; i++)
        s = s * 31 + a % b;
    return s;
}

/* `a[i] = f(a[i-1])`: loop-carried forwarding keeps a[i-1] in a register
 * and loads a[0] once in front of the loop. */
NOINLINE void prefix(long *a, long n) {
    for (long i = 1; i < n; i++)
        a[i] = a[i - 1] + 3;
}

NOINLINE void prefix2(long *a, const long *b, long n) {
    for (long i = 1; i < n; i++)
        a[i] = a[i - 1] + b[i];
}

NOINLINE void iir(double *y, const double *x, long n) {
    for (long i = 1; i < n; i++)
        y[i] = 0.5 * y[i - 1] + x[i];
}

/* An exit in front of the load: the load is not reached on every trip, so
 * it may not move even behind the guard. */
NOINLINE long load_behind_a_break(const long *p, const long *a, long n) {
    long s = 0;
    for (long i = 0; i < n; i++) {
        if (a[i] < 0)
            break;
        s += *p + a[i];
    }
    return s;
}

/* A division behind a condition of the body. */
NOINLINE long div_behind_a_test(long a, long b, const long *m, long n) {
    long s = 0;
    for (long i = 0; i < n; i++) {
        if (m[i])
            s += a / b;
        s ^= i;
    }
    return s;
}

/* The inner loop of a nest reads a field its own loop never changes (the
 * insertion walk of a binary tree keyed by `keys[i]`). */
struct node {
    struct node *left, *right;
    int key;
};

NOINLINE long tree_insert_all(struct node *pool, const int *keys, long n) {
    struct node *root = 0;
    long depth_sum = 0;
    for (long i = 0; i < n; i++) {
        struct node **slot = &root;
        pool[i].key = keys[i];
        pool[i].left = pool[i].right = 0;
        while (*slot) {
            depth_sum++;
            if (pool[i].key < (*slot)->key)
                slot = &(*slot)->left;
            else
                slot = &(*slot)->right;
        }
        *slot = &pool[i];
    }
    return depth_sum;
}

/* Two exits into one block: the writeback of a promoted value belongs on
 * the loop's own exit edge, not in the block the guard reaches too. */
NOINLINE long acc_then_read(long *p, long n) {
    for (long i = 0; i < n; i++)
        *p += i + 1;
    return n > 0 ? *p : -1;
}

/* A volatile accumulator stays in memory, every trip. */
NOINLINE void acc_volatile(volatile long *p, long n) {
    for (long i = 0; i < n; i++)
        *p += i;
}

/* ---- references the optimizer cannot rewrite --------------------------- */

static long vload(const long *p) { return *(const volatile long *)p; }
static void vstore(long *p, long v) { *(volatile long *)p = v; }

static void ref_acc_array(long *p, const long *a, long n) {
    for (volatile long i = 0; i < n; i++)
        vstore(p, vload(p) + (vload(&a[i]) ^ 5));
}

static void ref_prefix2(long *a, const long *b, long n) {
    for (volatile long i = 1; i < n; i++)
        vstore(&a[i], vload(&a[i - 1]) + vload(&b[i]));
}

/* ---- the harness ------------------------------------------------------- */

static int failures;

static void check(const char *what, long got, long want) {
    if (got != want) {
        printf("FAIL %s: got %ld, want %ld\n", what, got, want);
        failures++;
    }
}

static void *page(int prot) {
    long size = sysconf(_SC_PAGESIZE);
    void *p = mmap(0, (size_t)size, prot, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (p == MAP_FAILED) {
        perror("mmap");
        exit(2);
    }
    return p;
}

#define N 64

int main(void) {
    /* Reads and writes fault here; writes fault there. */
    void *none = page(PROT_NONE);
    void *readonly = page(PROT_READ);
    long a[N + 2], b[N + 2], want[N + 2];
    double x[N], y[N];
    static const long trips[] = {0, -1, LONG_MIN, 1, 2, 3, 7, N};

    for (int i = 0; i < N + 2; i++)
        a[i] = (long)i * 2654435761L % 1009 - 400;

    /* 1. Zero trips: nothing may be touched, nothing may trap. */
    for (int t = 0; t < 3; t++) {
        long n = trips[t];
        acc_field(none, n);
        acc_field(readonly, n);
        acc_array(none, none, n);
        acc_array(readonly, a, n);
        check("inv_load zero", inv_load(none, none, n), 0);
        check("inv_div by zero, zero trips", inv_div(7, 0, n), 0);
        check("inv_div overflow, zero trips", inv_div(LONG_MIN, -1, n), 0);
        check("inv_rem by zero, zero trips", (long)inv_rem(7, 0, n), 0);
        check("load_behind_a_break zero", load_behind_a_break(none, none, n), 0);
        check("div_behind_a_test zero", div_behind_a_test(1, 0, none, n), 0);
        check("acc_then_read zero", acc_then_read(none, n), -1);
        acc_volatile(none, n);
        /* The first element a prefix loop reads is a[0], and only if it
         * runs: n <= 1 runs nothing. */
        prefix(none, n);
        prefix(none, 1);
        prefix2(none, none, n);
        prefix2(readonly, none, 1);
        iir(none, none, n);
        iir(none, none, 1);
    }

    /* 2. A break on the first trip in front of the load, and a division
     *    whose condition is never true. */
    {
        long neg[4] = {-1, 5, 6, 7};
        long zeros[4] = {0, 0, 0, 0};
        check("load_behind_a_break taken", load_behind_a_break(none, neg, 4), 0);
        check("div_behind_a_test never", div_behind_a_test(1, 0, zeros, 4), 0 ^ 1 ^ 2 ^ 3);
        check("div_behind_a_test overflow never", div_behind_a_test(LONG_MIN, -1, zeros, 4), 0 ^ 1 ^ 2 ^ 3);
    }

    /* 3. Ordinary operands, every trip count. */
    for (unsigned t = 0; t < sizeof trips / sizeof *trips; t++) {
        long n = trips[t];
        struct counter c = {11, 100, 1.5};
        long acc = 1000, k = 3;
        unsigned long s;

        acc_field(&c, n);
        printf("acc_field n=%ld: %ld %ld %.1f\n", n, c.pad, c.count, c.mass);

        acc_array(&acc, a, n);
        printf("acc_array n=%ld: %ld\n", n, acc);

        printf("inv_load n=%ld: %ld\n", n, inv_load(&k, a, n));
        printf("inv_div n=%ld: %ld %ld\n", n, inv_div(1000003, 17, n), inv_div(-1000003, 17, n));
        printf("inv_rem n=%ld: %lu\n", n, inv_rem(1000003, 17, n));

        memcpy(b, a, sizeof b);
        prefix(b, n);
        s = 0;
        for (int i = 0; i < N; i++)
            s = s * 3 + (unsigned long)b[i];
        printf("prefix n=%ld: %lu\n", n, s);

        memcpy(b, a, sizeof b);
        prefix2(b, a, n);
        s = 0;
        for (int i = 0; i < N; i++)
            s = s * 3 + (unsigned long)b[i];
        printf("prefix2 n=%ld: %lu\n", n, s);

        for (int i = 0; i < N; i++) {
            x[i] = (double)a[i] / 8;
            y[i] = 1.0;
        }
        iir(y, x, n);
        printf("iir n=%ld: %.6f %.6f\n", n, y[n > 0 && n <= N ? n - 1 : 0], y[N - 1]);

        printf("load_behind_a_break n=%ld: %ld\n", n, load_behind_a_break(&k, a, n));
        printf("div_behind_a_test n=%ld: %ld\n", n, div_behind_a_test(977, 13, a, n));

        acc = 5;
        s = (unsigned long)acc_then_read(&acc, n);
        printf("acc_then_read n=%ld: %ld %ld\n", n, (long)s, acc);

        acc = 9;
        acc_volatile(&acc, n);
        printf("acc_volatile n=%ld: %ld\n", n, acc);
    }

    /* 4. Aliasing. The accumulator is an element of the array it sums;
     *    the two prefix arrays are the same array, shifted either way. */
    for (int at = 0; at < 8; at++) {
        memcpy(b, a, sizeof b);
        memcpy(want, a, sizeof want);
        acc_array(&b[at], b, 8);
        ref_acc_array(&want[at], want, 8);
        check("acc_array aliasing", memcmp(b, want, sizeof b), 0);
        printf("acc_array p=&a[%d]: %ld\n", at, b[at]);
    }
    for (int shift = -1; shift <= 1; shift++) {
        /* 24 trips: shifted by -1 the loop doubles its element each trip. */
        enum { TRIPS = 24 };
        memcpy(b, a, sizeof b);
        memcpy(want, a, sizeof want);
        prefix2(b + 1, b + 1 + shift, TRIPS);
        ref_prefix2(want + 1, want + 1 + shift, TRIPS);
        check("prefix2 aliasing", memcmp(b, want, sizeof b), 0);
        printf("prefix2 shift=%d: %ld %ld\n", shift, b[TRIPS / 2], b[TRIPS]);
    }

    /* 5. The nest. */
    {
        enum { NODES = 200 };
        static struct node pool[NODES];
        static int keys[NODES];
        unsigned state = 0x4d2b79f5u;
        for (int i = 0; i < NODES; i++) {
            state = state * 1664525u + 1013904223u;
            keys[i] = (int)(state >> 8 & 0xffff);
        }
        long none_inserted = tree_insert_all(pool, keys, 0);
        long one_inserted = tree_insert_all(pool, keys, 1);
        long all_inserted = tree_insert_all(pool, keys, NODES);
        printf("tree_insert_all: %ld %ld %ld\n", none_inserted, one_inserted, all_inserted);
    }

    if (failures) {
        printf("%d failures\n", failures);
        return 1;
    }
    printf("ok\n");
    return 0;
}
