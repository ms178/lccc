/* A small local copied from memory and then only read may be elided by the
 * inliner (`elide_readonly_local_memcpys`), but its readers must still see
 * the bytes the copy took, not whatever the source holds when they run.
 *
 *     E e = t[i];        // memcpy %e <- &t[i]: the copy the inliner elides
 *     t[i].a = 99;       // a write to the SOURCE after the copy
 *     return e.a;        // must read the copied value
 *
 * The elision proved the LOCAL read-only and then re-rooted its loads at the
 * source, so any write to the source between the copy and a read (a field
 * store, a store through an aliasing pointer, a loop-carried update) leaked
 * into the copy. `control` never writes the source and is the positive
 * control for the elision itself.
 *
 * GCC 16.2: "7 1 30 3 11 9".
 */
#include <stdio.h>

typedef struct {
    int a, b;
} E;

/* Field store to the source after the copy, same block. */
static inline __attribute__((always_inline)) int field_store(E *t, int i) {
    E e = t[i];
    t[i].a = 99;
    return e.a + e.b;
}

/* Store through a pointer that aliases the source. */
static inline __attribute__((always_inline)) int aliased(E *t, int *p) {
    E e = *t;
    *p = 50;
    return e.a;
}

/* The source is rewritten on every iteration after the copy; the read
 * happens later in the same iteration and on the exit path. */
static inline __attribute__((always_inline)) int loop_carried(E *t, int n) {
    E e = *t;
    int acc = 0;
    for (int k = 0; k < n; k++) {
        t->a = t->a * 2 + k;
        acc += e.a;
    }
    return acc + e.b;
}

/* By-value parameter: the inliner copies the argument into a local and
 * elides it; the caller's object is written inside the body. */
static inline __attribute__((always_inline)) int by_value(E e, E *src) {
    src->b = 1000;
    return e.a + e.b;
}

/* Positive control: no write to the source, the elision must still apply. */
static inline __attribute__((always_inline)) int control(const E *t) {
    E e = *t;
    return e.a + e.b;
}

/* A conditional read of the copy: the read must still see the snapshot when
 * the source was rewritten on the other path. */
static inline __attribute__((always_inline)) int conditional(E *t, int c) {
    E e = *t;
    if (c)
        t->a = 77;
    return c ? e.a + 8 : e.b;
}

__attribute__((noinline)) static int run_field_store(E *t, int i) { return field_store(t, i); }
__attribute__((noinline)) static int run_aliased(E *t, int *p) { return aliased(t, p); }
__attribute__((noinline)) static int run_loop(E *t, int n) { return loop_carried(t, n); }
__attribute__((noinline)) static int run_by_value(E *s) { return by_value(*s, s); }
__attribute__((noinline)) static int run_control(const E *t) { return control(t); }
__attribute__((noinline)) static int run_conditional(E *t, int c) { return conditional(t, c); }

int main(void) {
    E t[2] = {{1, 2}, {3, 4}};
    E u = {1, 9};
    E v = {9, 3};
    E w = {5, 6};
    E x = {1, 2};
    E y = {1, 9};
    int r1 = run_field_store(t, 1);  /* 3 + 4 */
    int r2 = run_aliased(&u, &u.a);  /* 1 */
    int r3 = run_loop(&v, 3);        /* 9 * 3 + 3 */
    int r4 = run_by_value(&x);       /* 1 + 2: the copy precedes the write */
    int r5 = run_control(&w);        /* 11 */
    int r6 = run_conditional(&y, 1); /* 1 + 8 */
    printf("%d %d %d %d %d %d\n", r1, r2, r3, r4, r5, r6);
    return 0;
}
