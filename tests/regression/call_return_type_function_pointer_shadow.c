/* Linux timer_migration.c: a callback parameter named `up` shadows the
 * global void up(struct semaphore *). Its return type, not the global's,
 * governs value contexts. Exercise boolean and floating ABI return paths. */
#include <stdio.h>

struct semaphore;
extern void up(struct semaphore *);
typedef _Bool (*predicate)(int *);
typedef double (*transform)(int *);
static _Bool positive(int *p) { return *p > 0; }
static double fraction(int *p) { return *p + 0.5; }

__attribute__((noinline)) static int run_bool(predicate up, int *p)
{
    return up(p) ? 11 : 22;
}
__attribute__((noinline)) static double run_double(transform up, int *p)
{
    return up(p) + 1.0;
}
__attribute__((noinline)) static int run_local(int *p)
{
    predicate up = positive;
    return (*up)(p) ? 33 : 44;
}
int main(void)
{
    int p = 2, n = -1;
    int ok = run_bool(positive, &p) == 11 && run_bool(positive, &n) == 22 &&
             run_double(fraction, &p) == 3.5 && run_local(&p) == 33;
    puts(ok ? "shadow returns: ok" : "shadow returns: FAIL");
    return !ok;
}
