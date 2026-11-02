/* Regression: loop store motion (loop_memory_promote) must keep the program's
 * observable stores and loads in order. It must not write on a zero-trip path,
 * and it must leave alone any loop whose memory traffic may observe the stored
 * location.
 *
 * Each case prints its result; check_loop_store_motion.sh compares the output
 * against GCC -O0. Its placement check reads the specific functions below, so
 * keep their names and shapes. */

#include <stdio.h>

int g_last = 12345;
int g_sum;
static const int g_ro = 7;
volatile int g_vol;

/* Store-only global: the final value is the last store, or the old value
 * when the loop never runs. */
__attribute__((noinline)) static void store_only(int n) {
    for (int i = 0; i < n; i++)
        g_last = i * 3 + 1;
}

/* Store-only to a local through an invariant pointer. */
__attribute__((noinline)) static int store_local(int n) {
    int local = -1;
    int *p = &local;
    for (int i = 0; i < n; i++)
        *p = i ^ 0x55;
    return local;
}

/* Do-while accumulator through a pointer parameter. The loop exits from its
 * latch, so the existing pass promotes it: the store moves after the loop. */
__attribute__((noinline)) static void acc_dowhile(int *out, int n) {
    int i = 0;
    do {
        *out += i;
        i++;
    } while (i < n);
}

/* Store whose value is computed from a loop-carried recurrence. */
__attribute__((noinline)) static void store_recurrence(int n) {
    int x = 3;
    for (int i = 0; i < n; i++) {
        x = x * 5 + i;
        g_last = x;
    }
}

/* Conditional store: not an every-iteration store, so it must not be moved. */
__attribute__((noinline)) static void store_conditional(int n) {
    for (int i = 0; i < n; i++)
        if (i & 1)
            g_last = i;
}

/* A load through an aliasing pointer must observe every store. */
__attribute__((noinline)) static int store_then_alias_load(int *p, int n) {
    int acc = 0;
    for (int i = 0; i < n; i++) {
        g_last = i;
        acc += *p;
    }
    return acc;
}

/* A store to a read-only object on a zero-trip path must not execute. */
__attribute__((noinline)) static void store_readonly_zero_trip(int n) {
    int *p = (int *)(unsigned long)&g_ro;
    for (int i = 0; i < n; i++)
        *p = i;
}

/* A call in the loop may observe the store: must not be moved. */
__attribute__((noinline)) static void observe(void) { g_sum += g_last; }

__attribute__((noinline)) static void store_with_call(int n) {
    for (int i = 0; i < n; i++) {
        g_last = i;
        observe();
    }
}

/* Volatile store: observable on every iteration, must not be moved. */
__attribute__((noinline)) static void store_volatile(int n) {
    for (int i = 0; i < n; i++)
        g_vol = i;
}

int main(void) {
    store_only(0);
    printf("store_only(0) g_last=%d\n", g_last);
    store_only(5);
    printf("store_only(5) g_last=%d\n", g_last);
    store_only(1);
    printf("store_only(1) g_last=%d\n", g_last);

    printf("store_local(0)=%d store_local(7)=%d\n", store_local(0), store_local(7));

    int total = 100;
    acc_dowhile(&total, 4);
    printf("dowhile total=%d\n", total);

    store_recurrence(0);
    printf("recurrence(0) g_last=%d\n", g_last);
    store_recurrence(6);
    printf("recurrence(6) g_last=%d\n", g_last);

    store_conditional(0);
    printf("conditional(0) g_last=%d\n", g_last);
    store_conditional(6);
    printf("conditional(6) g_last=%d\n", g_last);

    g_last = 0;
    /* Separate statements: the call modifies g_last, so reading it in the
     * same argument list would be unsequenced. */
    int acc = store_then_alias_load(&g_last, 4);
    printf("alias_load(4)=%d g_last=%d\n", acc, g_last);

    store_readonly_zero_trip(0);
    printf("readonly_zero_trip ok g_ro=%d\n", g_ro);

    g_sum = 0;
    store_with_call(4);
    printf("with_call g_last=%d g_sum=%d\n", g_last, g_sum);

    store_volatile(3);
    printf("volatile g_vol=%d\n", g_vol);
    return 0;
}
