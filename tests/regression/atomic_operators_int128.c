/* The operators on a 16-byte atomic integer (see atomic_operators.c).
 *
 * x86-64 has no 16-byte locked instruction short of CMPXCHG16B, so every
 * access goes through libatomic's `_16` entry points, as GCC's does: a load
 * that read the halves one after the other could see one half of another
 * thread's update, and an update written as load, add, store lost it. */
#include <pthread.h>
#include <stdio.h>

typedef unsigned __int128 u128;
static _Atomic u128 wide;
static _Atomic __int128 swide;
static _Atomic u128 torn;
static volatile int one = 1, three = 3;

static void show(const char *label, u128 v) {
    printf("%s: %016llx%016llx\n", label, (unsigned long long)(v >> 64), (unsigned long long)v);
}

#define THREADS 4
#define ROUNDS 20000
/* Every thread adds the same value to both halves: the halves of a value
 * that was never torn are equal. */
#define STEP (((u128)1 << 64) + 1)

static int torn_reads;
static void *worker(void *arg) {
    (void)arg;
    for (int i = 0; i < ROUNDS; i++) {
        torn += STEP;
        u128 seen = torn;
        if ((unsigned long long)(seen >> 64) != (unsigned long long)seen)
            __atomic_fetch_add(&torn_reads, 1, __ATOMIC_RELAXED);
        swide--;
        wide ^= (u128)1 << 100;
        ++wide;
    }
    return 0;
}

int main(void) {
    wide = ~(u128)0;
    show("post-inc", wide++);
    show("wrapped", wide);
    show("add", wide += ((u128)three << 70) + 5);
    show("mul", wide *= three);
    show("shift", wide <<= one);
    show("sub", wide -= (u128)1 << 71);
    show("or", wide |= 0xf0);
    show("and", wide &= ~(u128)0x30);
    show("div", wide /= three);
    swide = -1;
    show("signed pre-dec", (u128)--swide);
    show("signed shift", (u128)(swide >>= one));
    show("assign", wide = (u128)three << 96);
    show("value", wide);

    pthread_t tid[THREADS];
    wide = 0;
    swide = 0;
    for (int i = 0; i < THREADS; i++)
        pthread_create(&tid[i], 0, worker, 0);
    for (int i = 0; i < THREADS; i++)
        pthread_join(tid[i], 0);
    show("threads torn", torn);
    show("threads wide", wide);
    show("threads swide", (u128)swide);
    printf("torn reads: %d\n", torn_reads);
    return torn != STEP * THREADS * ROUNDS || wide != (u128)THREADS * ROUNDS ||
           swide != -(__int128)THREADS * ROUNDS || torn_reads != 0;
}
