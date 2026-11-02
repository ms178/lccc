/* control flow — loops, switches (incl. ranges), goto,
 * recursion, TCE (tail-call elimination), short-circuit, ternary. */
#include <stdio.h>

static int fib(int n) { return n < 2 ? n : fib(n-1) + fib(n-2); }
static int fact_tail(int n, int acc) { return n <= 1 ? acc : fact_tail(n-1, acc*n); }
static int sum_to(int n) { int s = 0; for (int i = 1; i <= n; i++) s += i; return s; }
static int classify(int x) {
    switch (x) {
    case 0: return 100;
    case 1: case 2: return 200;
    case 10 ... 20: return 300;      /* GNU case range */
    default: return 400;
    }
}
static int countdown(int n) { int c = 0; while (n > 0) { c++; n--; } return c; }
static int do_loop(int n) { int c = 0; do { c += n; n--; } while (n > 0); return c; }

int main(void) {
    if (fib(10) != 55) return 1;
    if (fib(20) != 6765) return 2;
    if (fact_tail(10, 1) != 3628800) return 3;
    if (sum_to(100) != 5050) return 4;

    if (classify(0) != 100) return 5;
    if (classify(1) != 200 || classify(2) != 200) return 6;
    if (classify(15) != 300) return 7;
    if (classify(21) != 400) return 8;
    if (classify(-5) != 400) return 9;

    if (countdown(10) != 10) return 10;
    if (countdown(0) != 0) return 11;
    if (do_loop(5) != 15) return 12;

    /* short-circuit */
    int calls = 0;
    int r = 0;
    r = (calls++ == 0) && (calls++ == 1);
    if (r != 1 || calls != 2) return 13;
    calls = 0;
    r = (calls++ == 0) || (calls++ == 1);
    if (r != 1 || calls != 1) return 14;   /* second not evaluated */
    r = 0 && (calls = 99);
    if (calls != 1) return 15;             /* RHS not evaluated */

    /* ternary */
    int t = 5 > 3 ? 10 : 20;
    if (t != 10) return 16;
    t = 5 < 3 ? 10 : 20;
    if (t != 20) return 17;

    /* goto / labels */
    int g = 0;
    goto lbl;
lbl2:
    g += 2;
    goto end;
lbl:
    g = 1;
    goto lbl2;
end:
    if (g != 3) return 18;

    /* comma operator */
    int a, b;
    a = (b = 3, b + 4);
    if (a != 7 || b != 3) return 19;

    /* loops with continue/break */
    int sum = 0, hits = 0;
    for (int i = 0; i < 20; i++) {
        if (i % 2) continue;
        if (i > 10) break;
        sum += i; hits++;
    }
    if (sum != 30 || hits != 6) return 20;  /* 0+2+4+6+8+10 */

    printf("OK control_flow\n");
    return 0;
}
