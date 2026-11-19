/* A call is not run in an execution that would not have reached it.
 *
 * Loop-invariant code motion hoisted a read-none call without pointer
 * arguments into whatever block precedes its loop -- including one that also
 * branches around the loop -- on the reasoning that such a call cannot fault.
 * A callee that divides can; so can one that traps, or parks.  With the loops
 * below bottom-tested, the block ahead of each is the very test that makes the
 * call legal, and the call ran above it:
 *
 *   sum_quot(3, 7, 0)   SIGFPE        (GCC 16.2, Clang 23.1.5: -1)
 *   sum_checked(3, 9)   SIGILL        (-1)
 *   sum_settle(3, -5)   never returns (-1)
 *
 * `sum_required` is the same hazard inside the loop: the call that makes the
 * division legal comes first and may not return, and the division was hoisted
 * above it.
 *
 * A callee known only by its declaration is no different.  glibc declares
 * `div` `const`, and it divides exactly as `quot` does; "const, and from the
 * library" was taken to mean "defined for every argument":
 *
 *   sum_div(3, 7, 0)    SIGFPE        (-1)
 *
 * Every escape is caught and reported as an exit code; a 1 s timer turns a
 * hang into one as well.  The arguments go through a volatile so that the
 * loops are compiled as loops rather than evaluated at compile time.
 */
#include <setjmp.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/time.h>

static sigjmp_buf back;
static volatile int zero;
enum { REFUSED = 1000 };
#define V(x) ((x) + zero)

static void on_signal(int sig)
{
    siglongjmp(back, sig);
}

__attribute__((noinline)) static int quot(int a, int b)
{
    return a / b;
}

__attribute__((noinline)) static int checked(int i, int n)
{
    if (i >= n)
        __builtin_trap();
    return i * 2;
}

__attribute__((noinline)) static int settle(int v)
{
    if (v < 0)
        for (;;) {
        }
    return v + 1;
}

__attribute__((noinline)) static int sum_quot(int trips, int a, int b)
{
    int s = 0, j = 0;
    if (b == 0)
        return -1;
    do
        s += quot(a, b);
    while (++j < trips);
    return s;
}

__attribute__((noinline)) static int sum_checked(int trips, int k)
{
    int s = 0, j = 0;
    if (k >= 4)
        return -1;
    do
        s += checked(k, 4);
    while (++j < trips);
    return s;
}

__attribute__((noinline)) static int sum_settle(int trips, int v)
{
    int s = 0, j = 0;
    if (v < 0)
        return -1;
    do
        s += settle(v);
    while (++j < trips);
    return s;
}

__attribute__((noinline)) static void require_nonzero(int b)
{
    if (b == 0)
        siglongjmp(back, REFUSED);
}

__attribute__((noinline)) static int sum_required(int trips, int a, int b)
{
    int s = 0, j = 0;
    do {
        require_nonzero(b);
        s += quot(a, b);
    } while (++j < trips);
    return s;
}

__attribute__((noinline)) static int sum_div(int trips, int a, int b)
{
    int s = 0, j = 0;
    if (b == 0)
        return -1;
    do
        s += div(a, b).quot;
    while (++j < trips);
    return s;
}

__attribute__((noinline)) static int sum_div_required(int trips, int a, int b)
{
    int s = 0, j = 0;
    do {
        require_nonzero(b);
        s += div(a, b).quot;
    } while (++j < trips);
    return s;
}

int main(void)
{
    static const int signals[] = {SIGILL, SIGTRAP, SIGABRT, SIGSEGV, SIGFPE, SIGALRM};
    struct itimerval watchdog = {{0, 0}, {1, 0}};
    struct sigaction action;
    int sig;

    action.sa_handler = on_signal;
    action.sa_flags = 0;
    sigemptyset(&action.sa_mask);
    for (unsigned i = 0; i < sizeof signals / sizeof signals[0]; i++)
        sigaction(signals[i], &action, 0);
    setitimer(ITIMER_REAL, &watchdog, 0);

    sig = sigsetjmp(back, 1);
    if (sig != 0) {
        printf("a guarded call ran anyway: signal %d\n", sig);
        return 1;
    }
    /* The guarded executions: none of the three calls may run. */
    if (sum_quot(V(3), V(7), V(0)) != -1 || sum_checked(V(3), V(9)) != -1 ||
        sum_settle(V(3), V(-5)) != -1)
        return 2;
    if (sum_div(V(3), V(7), V(0)) != -1)
        return 7;
    /* The executions that do enter the loop still compute the same sums. */
    if (sum_quot(V(3), V(7), V(2)) != 9 || sum_checked(V(3), V(1)) != 6 ||
        sum_settle(V(3), V(4)) != 15)
        return 3;
    if (sum_required(V(3), V(7), V(2)) != 9)
        return 4;
    if (sum_div(V(3), V(7), V(2)) != 9 || sum_div_required(V(3), V(7), V(2)) != 9)
        return 8;

    sig = sigsetjmp(back, 1);
    if (sig == 0) {
        printf("the refused division returned %d\n", sum_required(V(3), V(7), V(0)));
        return 5;
    }
    if (sig != REFUSED) {
        printf("the division ran before the call that refuses it: signal %d\n", sig);
        return 6;
    }

    sig = sigsetjmp(back, 1);
    if (sig == 0) {
        printf("the refused div() returned %d\n", sum_div_required(V(3), V(7), V(0)));
        return 9;
    }
    if (sig != REFUSED) {
        printf("div() ran before the call that refuses it: signal %d\n", sig);
        return 10;
    }

    puts("ok");
    return 0;
}
