/* Two switch shapes of kernel code, and what each must not leave behind.
 *
 * `case X: break;` and a `default:` that only breaks are empty blocks in
 * front of the join.  A `switch` whose cases name every value its operand can
 * have has a default nothing selects.  LCCC emitted a `jmp` behind the jump
 * table for the first (Linux `ftrace_bug`, `l1tf_apply_mitigation`) and the
 * whole default block for the second (`rcu_preempt_ctxt_queue`: a `ud2` and a
 * `__bug_table` entry); objtool reported both as `unreachable instruction`.
 *
 * The values are what this file checks; the dead code is held to account by
 * tests/regression/check_kernel_driver_shapes.py.
 */
#include <stdio.h>

volatile int bug_type;
static int reported;
__attribute__((noinline)) static void say(const char *what) { reported += what[0]; }

__attribute__((noinline)) int report(int failed)
{
    say("-");
    switch (bug_type) {
    case 0:
        break;
    case 1:
        say("init");
        break;
    case 2:
        say("nop");
        break;
    case 3:
        say("call");
        break;
    case 4:
        say("update");
        break;
    default:
        break;
    }
    return failed ? reported : -reported;
}

/* Four one-bit facts, sixteen cases, and a default for the impossible. */
__attribute__((noinline)) int queue(int gp_tasks, int exp_tasks, int gp_blkd, int exp_blkd)
{
    int state = (gp_tasks ? 1 : 0) + (exp_tasks ? 2 : 0) + (gp_blkd ? 4 : 0) + (exp_blkd ? 8 : 0);
    int where;
    switch (state) {
    case 0:
    case 2:
    case 1:
    case 3:
    case 1 + 8:
    case 3 + 8:
        where = 10;
        break;
    case 8:
    case 4 + 8:
    case 1 + 4 + 8:
    case 2 + 4 + 8:
    case 1 + 2 + 4 + 8:
        where = 20;
        break;
    case 2 + 8:
    case 4:
    case 2 + 4:
        where = 30;
        break;
    case 1 + 4:
    case 1 + 2 + 4:
        where = 40;
        break;
    default:
        say("impossible");
        where = -1;
        break;
    }
    return where + state;
}

/* A dense switch over a masked value that returns from every case. */
__attribute__((noinline)) int quadrant(unsigned x)
{
    switch (x & 3) {
    case 0:
        return 100;
    case 1:
        return 111;
    case 2:
        return 122;
    case 3:
        return 133;
    }
    return -1;
}

/* Cases that share the default's target with others that do not. */
__attribute__((noinline)) int sparse(int x)
{
    int r = 0;
    switch (x) {
    case 1:
        break;
    case 2:
        r = 20;
        break;
    case 5:
        break;
    case 9:
        r = 90;
        break;
    case 40:
        break;
    default:
        break;
    }
    return r + x;
}

int main(void)
{
    for (int t = 0; t < 7; t++) {
        bug_type = t;
        printf("report %d %d\n", t, report(t & 1));
    }
    for (int s = 0; s < 16; s++)
        printf("queue %d %d\n", s, queue(s & 1, s & 2, s & 4, s & 8));
    for (unsigned x = 0; x < 6; x++)
        printf("quadrant %u %d\n", x, quadrant(x));
    for (int x = -1; x < 42; x += (x < 10 ? 1 : 15))
        printf("sparse %d %d\n", x, sparse(x));
    printf("reported %d\n", reported);
    return 0;
}
