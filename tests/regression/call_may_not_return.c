/* A call that may not come back is not removable, whatever it leaves alone.
 *
 * "No side effects" is what lets a compiler delete a call whose result is
 * unused, fold one whose result is known, and run one earlier than written.
 * It is not enough: a trap and a loop that does not end are both observable,
 * and neither writes memory.  LCCC inferred `const` for `check` and `park`
 * below from their memory effects alone and deleted both calls at -O1 and
 * above -- the bounds check stopped checking (`get(a, 5, 4)` read `a[0]`), and
 * the program walked past a function that never returns.  GCC keeps such a
 * function "looping const" and LLVM withholds `willreturn`; both keep the call.
 *
 * Three more ways to lose the same call, each tried below:
 *   - `checked` and `parked` return 0 whenever they return.  Replacing the
 *     call by that 0 deletes it just as well (-O2 and above).
 *   - `park(1)` with a constant argument was dropped as a call to an empty
 *     function: no instruction in it does anything (-O2 and above).
 *   - `need(p); sum += *p;` in a loop: the load is invariant, and above the
 *     check it faults where the check was supposed to trap.
 *
 * C11 6.8.5p6 allows assuming that a loop ends only when its controlling
 * expression is not a constant; `for (;;)` is not such a loop.
 *
 * The escapes are caught, so the test reports instead of dying: the trap must
 * arrive as a signal, and the parked call must still be running when a 50 ms
 * timer fires.  Each is tried with constant arguments, which the compiler may
 * evaluate ahead of time, and with arguments it cannot see through.
 */
#include <setjmp.h>
#include <signal.h>
#include <stdio.h>
#include <sys/time.h>

static sigjmp_buf back;
static volatile int zero;
#define V(x) ((x) + zero)

static void on_signal(int sig)
{
    siglongjmp(back, sig);
}

/* A bounds check kept out of line: its only effect is to stop the program. */
__attribute__((noinline)) static void check(unsigned i, unsigned n)
{
    if (i >= n)
        __builtin_trap();
}

__attribute__((noinline)) static int get(const int *a, unsigned i, unsigned n)
{
    check(i, n);
    return a[i < n ? i : 0];
}

__attribute__((noinline)) static void park(int stop)
{
    if (stop)
        for (;;) {
        }
}

__attribute__((noinline)) static int after_park(int stop)
{
    park(stop);
    return 7;
}

/* The same two with a result: 0 on every return is not the constant 0. */
__attribute__((noinline)) static int checked(unsigned i, unsigned n)
{
    if (i >= n)
        __builtin_trap();
    return 0;
}

__attribute__((noinline)) static int get_checked(const int *a, unsigned i, unsigned n)
{
    if (checked(i, n))
        return -1;
    return a[i < n ? i : 0];
}

__attribute__((noinline)) static int parked(int stop)
{
    if (stop)
        for (;;) {
        }
    return 0;
}

__attribute__((noinline)) static int after_parked(int stop)
{
    return parked(stop) ? -1 : 7;
}

/* A check ahead of a load, every iteration: the load waits for it. */
__attribute__((noinline)) static void need(const int *p)
{
    if (!p)
        __builtin_trap();
}

__attribute__((noinline)) static int sum_through(const int *p, int trips)
{
    int sum = 0, j = 0;
    do {
        need(p);
        sum += *p;
    } while (++j < trips);
    return sum;
}

/* The trap is one of these; a fault or the timer means the check was lost. */
static int trapped(int sig)
{
    return sig == SIGILL || sig == SIGTRAP || sig == SIGABRT;
}

static void arm_timer(void)
{
    struct itimerval soon = {{0, 0}, {0, 50000}};
    setitimer(ITIMER_REAL, &soon, 0);
}

int main(void)
{
    static const int a[4] = {1, 2, 3, 4};
    static const int signals[] = {SIGILL, SIGTRAP, SIGABRT, SIGSEGV, SIGBUS, SIGFPE, SIGALRM};
    struct sigaction action;
    int sig;

    action.sa_handler = on_signal;
    action.sa_flags = 0;
    sigemptyset(&action.sa_mask);
    for (unsigned i = 0; i < sizeof signals / sizeof signals[0]; i++)
        sigaction(signals[i], &action, 0);

    /* In range: every helper returns, and so do their callers. */
    if (get(a, 1, 4) != 2 || after_park(0) != 7)
        return 1;
    if (get(a, V(2), V(4)) != 3 || after_park(V(0)) != 7)
        return 1;
    if (get_checked(a, 1, 4) != 2 || after_parked(0) != 7)
        return 1;
    if (get_checked(a, V(2), V(4)) != 3 || after_parked(V(0)) != 7)
        return 1;
    if (sum_through(a + V(1), V(3)) != 6)
        return 1;

    sig = sigsetjmp(back, 1);
    if (sig == 0) {
        int value = get(a, 5, 4);
        printf("the check did not stop the program: read %d\n", value);
        return 2;
    }
    if (!trapped(sig))
        return 3;

    sig = sigsetjmp(back, 1);
    if (sig == 0) {
        int value = get(a, V(5), V(4));
        printf("the check did not stop the program: read %d\n", value);
        return 4;
    }
    if (!trapped(sig))
        return 5;

    sig = sigsetjmp(back, 1);
    if (sig == 0) {
        arm_timer();
        printf("the parked call returned %d\n", after_park(1));
        return 6;
    }
    if (sig != SIGALRM)
        return 7;

    sig = sigsetjmp(back, 1);
    if (sig == 0) {
        arm_timer();
        printf("the parked call returned %d\n", after_park(V(1)));
        return 8;
    }
    if (sig != SIGALRM)
        return 9;

    sig = sigsetjmp(back, 1);
    if (sig == 0) {
        int value = get_checked(a, 5, 4);
        printf("the check with a result did not stop the program: read %d\n", value);
        return 10;
    }
    if (!trapped(sig))
        return 11;

    sig = sigsetjmp(back, 1);
    if (sig == 0) {
        int value = get_checked(a, V(5), V(4));
        printf("the check with a result did not stop the program: read %d\n", value);
        return 12;
    }
    if (!trapped(sig))
        return 13;

    sig = sigsetjmp(back, 1);
    if (sig == 0) {
        arm_timer();
        printf("the parked call with a result returned %d\n", after_parked(1));
        return 14;
    }
    if (sig != SIGALRM)
        return 15;

    sig = sigsetjmp(back, 1);
    if (sig == 0) {
        arm_timer();
        printf("the parked call with a result returned %d\n", after_parked(V(1)));
        return 16;
    }
    if (sig != SIGALRM)
        return 17;

    sig = sigsetjmp(back, 1);
    if (sig == 0) {
        int value = sum_through((const int *)(long)V(0), V(3));
        printf("the check did not stop the loop: summed %d\n", value);
        return 18;
    }
    if (!trapped(sig)) {
        printf("the load ran ahead of its check: signal %d\n", sig);
        return 19;
    }

    puts("ok");
    return 0;
}
