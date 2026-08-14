/*
 * short-circuit evaluation order with side effects.
 *
 * The v9 if-conversion change keeps `||` diamonds branchy and the flag-fusion
 * change skips boolean materialization for branch-only conditions. Both must
 * preserve C's short-circuit semantics: the right operand of `&&`/`||` is NOT
 * evaluated when the left operand decides the result, and both arms of a
 * `cond ? a : b` are evaluated exactly when selected.
 *
 * `counter` and `seq` are global so every side effect is observable; the
 * printed values are deterministic and diffed against GCC.
 */
#include <stdio.h>

static int counter = 0;
static int seq = 0;

static int bump(void) { counter++; return (counter & 1); }       /* returns 1,0,1,0,... */
static int thrice(void) { counter += 3; return 1; }

static int or_lhs_true(void) { return (1) || bump(); }           /* bump NOT called */
static int or_lhs_false(void) { return (0) || bump(); }          /* bump called */
static int and_lhs_true(void) { return (1) && bump(); }          /* bump called */
static int and_lhs_false(void) { return (0) && bump(); }         /* bump NOT called */
static int nested_or(void) { return (1) || (0 && thrice()); }    /* nothing called */
static int nested_and(void) { return (1) && (1) && bump(); }     /* bump called once */
static int or_with_call(void) { return thrice() || bump(); }     /* thrice called, bump not */

static int cond_ternary(int x) { return (x > 0) ? thrice() : bump(); }

static int branch_or(int x) { if (x > 100 || x < -100) return 7; return 9; }
static int branch_and(int x) { if (x > 0 && x < 10) return 3; return 4; }

int main(void) {
    int r = 0, c = 0;
    counter = 0;
    r += or_lhs_true();   c += counter;
    r += or_lhs_false();  c += counter;
    r += and_lhs_true();  c += counter;
    r += and_lhs_false(); c += counter;
    r += nested_or();     c += counter;
    r += nested_and();    c += counter;
    r += or_with_call();  c += counter;
    printf("%d %d\n", r, c);

    counter = 0; seq = 0;
    r = 0; c = 0;
    r += cond_ternary(5); c += counter;
    r += cond_ternary(-5); c += counter;
    printf("%d %d\n", r, c);

    r = 0;
    r += branch_or(150); r += branch_or(0);
    r += branch_and(5); r += branch_and(50);
    printf("%d\n", r);
    return 0;
}
