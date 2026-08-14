/*
 * v12 regression: flat short-circuit lowering.
 *
 * a || b || c and a && b && c must keep C's left-to-right short-circuit
 * evaluation order, stop at the first decisive operand, and produce 0/1 —
 * for every nesting shape (flat, mixed &&/||, parens), with constants in
 * every position and observable global side effects.
 */
#include <stdio.h>

static int counter;
static int bump(void) { counter++; return counter & 3; }     /* 1,2,3,0,1,2,3,0,... */
static int never(void) { counter += 100; return 0; }
static int always(void) { counter += 1000; return 1; }

static int or3(int a, int b, int c) { return a || b || c; }
static int and3(int a, int b, int c) { return a && b && c; }
static int or4(int a, int b, int c, int d) { return a || b || c || d; }
static int mixed(int a, int b, int c) { return a || (b && c); }
static int mixed2(int a, int b, int c) { return (a && b) || c; }
static int mixed3(int a, int b, int c) { return a && (b || c); }
static int mixed4(int a, int b, int c) { return (a || b) && c; }

/* constants in every position */
static int c1(int x) { return 1 || x; }            /* true, no eval of x */
static int c2(int x) { return 0 || x; }            /* == bool(x) */
static int c3(int x) { return x || 1; }            /* eval x, then 1 */
static int c4(int x) { return x || 0; }            /* == bool(x) */
static int c5(int x) { return 0 && x; }
static int c6(int x) { return 1 && x; }
static int c7(int x) { return x && 0; }
static int c8(int x) { return x && 1; }
static int c9(int x) { return 0 || 0 || x || 1; }  /* leading 0s dropped, trailing 1 */
static int c10(int x) { return 1 && 1 && x && 0; } /* leading 1s dropped, trailing 0 */

/* side-effecting operands in every position */
static int s1(void) { return 0 || bump() || never(); }
static int s2(void) { return 1 && bump() && never(); }
static int s3(void) { return bump() || bump() || bump(); }
static int s4(void) { return never() || always() || never(); }
static int s5(void) { return bump() && (bump() || always()); }

static int seq;
static int tick(void) { return ++seq; }
static int seq_or(int n) { return (tick() || tick() || tick()) + n; }  /* only 1st tick runs */

int main(void) {
    int r = 0;
    r += or3(0, 0, 0); r += or3(0, 0, 1); r += or3(0, 1, 0); r += or3(1, 0, 0);
    r += and3(0, 0, 0); r += and3(1, 1, 1); r += and3(1, 1, 0); r += and3(0, 1, 1);
    r += or4(0, 0, 0, 1); r += or4(0, 0, 0, 0);
    r += mixed(0, 1, 1); r += mixed(1, 0, 0); r += mixed(0, 1, 0);
    r += mixed2(1, 1, 0); r += mixed2(0, 0, 1); r += mixed2(0, 0, 0);
    r += mixed3(1, 0, 0); r += mixed3(1, 1, 0); r += mixed3(0, 1, 1);
    r += mixed4(1, 1, 1); r += mixed4(0, 1, 1); r += mixed4(1, 0, 1);

    counter = 0;
    r += c1(bump()); r += c2(bump()); r += c3(bump()); r += c4(bump());
    r += c5(bump()); r += c6(bump()); r += c7(bump()); r += c8(bump());
    r += c9(bump()); r += c10(bump());
    printf("%d %d\n", r, counter);

    counter = 0;
    r = s1() + s2() + s3() + s4() + s5();
    printf("%d %d\n", r, counter);

    seq = 0;
    r = seq_or(100);
    printf("%d %d\n", r, seq);
    return 0;
}
