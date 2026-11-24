/* An atomic object is read where the program reads it.
 *
 * `while (!flag) ;` on an atomic flag is how one thread waits for another,
 * and it only works if every trip reads the flag. The types here carried no
 * trace of `_Atomic`, so the flag was memory like any other: the loop has
 * no store and no call, the load was loop-invariant, LICM hoisted it, and
 * the waiter span on a register for ever. All four ways of naming an atomic
 * object behaved so: a typedef name (`atomic_bool`), the qualifier on the
 * object, a member, and the target of a pointer.
 *
 * Each waiter below is released by a second thread twenty milliseconds
 * after it has entered its loop, so the flag is still clear when a hoisted
 * load would read it. A waiter that never sees its flag gives up after
 * 2^33 trips, so the failure is a line of output, not a hung test.
 *
 * (What makes an atomic access visible to the optimizer is that the parser
 * records `volatile` wherever it records `_Atomic`; see
 * `ParsedDeclAttrs::set_atomic`. The assembly of the same loops is checked
 * in tests/regression/check_volatile_access_semantics.sh.)
 */
#include <pthread.h>
#include <sched.h>
#include <stdatomic.h>
#include <stdio.h>
#include <unistd.h>

#define NOINLINE __attribute__((noinline))

/* 64 bits wherever this is compiled: a count of 2^33 does not fit a `long`
 * of 32. */
typedef unsigned long long count_t;
#define GIVE_UP ((count_t)1 << 33)

struct control {
    long pad;
    _Atomic int stop;
    count_t work;
};

static atomic_bool flag_typedef;
static _Atomic int flag_named;
static struct control ctl;
static _Atomic long flag_pointee;

/* The waiter that is in its loop, 1 to 4; the setter follows it. */
static atomic_int waiting;

NOINLINE static count_t wait_typedef(void)
{
    count_t spins = 0;
    while (!flag_typedef && ++spins < GIVE_UP)
        ;
    return spins;
}

NOINLINE static count_t wait_named(void)
{
    count_t spins = 0;
    while (!flag_named && ++spins < GIVE_UP)
        ;
    return spins;
}

NOINLINE static count_t wait_member(struct control *c)
{
    count_t spins = 0;
    /* The store in the loop is to another member: it does not excuse the
     * load of `stop` from being repeated, and must not be what repeats it. */
    while (!c->stop && ++spins < GIVE_UP)
        c->work += 3;
    return spins;
}

NOINLINE static count_t wait_pointee(_Atomic long *flag)
{
    count_t spins = 0;
    while (!*flag && ++spins < GIVE_UP)
        ;
    return spins;
}

static void *setter(void *unused)
{
    (void)unused;
    for (int which = 1; which <= 4; which++) {
        while (atomic_load(&waiting) < which)
            sched_yield();
        usleep(20000);
        switch (which) {
        case 1: flag_typedef = 1; break;
        case 2: flag_named = 1; break;
        case 3: ctl.stop = 1; break;
        default: flag_pointee = 1; break;
        }
    }
    return 0;
}

static void report(const char *what, count_t spins)
{
    printf("%-34s %s\n", what, spins < GIVE_UP ? "saw the flag" : "never saw the flag");
}

int main(void)
{
    pthread_t thread;
    count_t spins[4];

    if (pthread_create(&thread, 0, setter, 0) != 0) {
        puts("no second thread: nothing to test");
        return 0;
    }
    atomic_store(&waiting, 1);
    spins[0] = wait_typedef();
    atomic_store(&waiting, 2);
    spins[1] = wait_named();
    atomic_store(&waiting, 3);
    spins[2] = wait_member(&ctl);
    atomic_store(&waiting, 4);
    spins[3] = wait_pointee(&flag_pointee);
    pthread_join(thread, 0);

    report("atomic_bool, a typedef name:", spins[0]);
    report("_Atomic int, the object:", spins[1]);
    report("_Atomic int, a member:", spins[2]);
    report("_Atomic long, behind a pointer:", spins[3]);
    /* The member's neighbour was written once a trip, no more. */
    printf("work per trip: %llu\n", spins[2] ? ctl.work / spins[2] : 0);
    return 0;
}
