/* Regression: condition lowering of && / || in branch contexts must produce
 * correct short-circuit evaluation AND side-effect ordering. Before the
 * branch-chain lowering, each comparison in a condition was materialized as a
 * setcc/movzbl boolean and merged through a select; this test verifies the
 * direct-branch lowering preserves semantics exactly (evaluation order,
 * short-circuiting, side effects), not just the boolean result.
 */
#include <stdint.h>
#include <stdio.h>

static int g_side = 0;
static int side(int v) { g_side++; return v; }

/* Plain comparison chains */
static int f1(int a, int b, int c, int d) {
    if (a != b || c != d) return 1;
    return 2;
}
static int f2(int a, int b, int c, int d) {
    if (a == b && c == d) return 3;
    return 4;
}
static int f3(int a, int b, int c, int d) {
    if ((a != b && c != d) || (a > d && b < c)) return 5;
    return 6;
}
/* Side effects must be evaluated in order and short-circuit */
static int f4(int a, int b) {
    g_side = 0;
    if (a == 0 || side(b) == 0) return 10 + g_side;
    return 20 + g_side;
}
static int f5(int a, int b) {
    g_side = 0;
    if (a != 0 && side(b) == 0) return 30 + g_side;
    return 40 + g_side;
}
/* while/do-while with logical conditions */
static int f6(int n) {
    int i = 0, acc = 0;
    while (i < n && acc < 50) { acc += i; i++; }
    return acc;
}
static int f7(int n) {
    int i = 0, acc = 0;
    do { acc += i; i++; } while (i < n && acc < 10);
    return acc;
}
/* ternary with logical condition */
static int f8(int a, int b) {
    return (a > 0 || b > 0) ? 7 : 8;
}

int main(void) {
    if (f1(1, 2, 3, 3) != 1) return 1;  /* a!=b true  -> then */
    if (f1(1, 1, 3, 4) != 1) return 2;  /* a==b, c!=d -> then */
    if (f1(1, 1, 3, 3) != 2) return 3;  /* both equal -> else */
    if (f2(1, 1, 3, 3) != 3) return 4;
    if (f2(1, 2, 3, 3) != 4) return 5;
    if (f3(1, 2, 1, 2) != 5) return 6;  /* (1!=2 && 1!=2) true -> then */
    if (f3(1, 2, 3, 3) != 6) return 7;  /* first group false (3==3) */
    if (f4(0, 0) != 10) return 8;       /* a==0 short-circuits side() */
    if (g_side != 0) return 9;
    if (f4(1, 5) != 20 + 1) return 10;  /* a!=0, side(5)!=0 -> else; side effect counted */
    if (g_side != 1) return 11;
    if (f4(1, 0) != 10 + 1) return 12;  /* a!=0, side(0)==0 -> then */
    if (g_side != 1) return 13;
    if (f5(0, 0) != 40) return 14;      /* a==0 short-circuits */
    if (g_side != 0) return 15;
    if (f5(1, 0) != 30 + 1) return 16;
    if (f5(1, 9) != 40 + 1) return 17;
    if (f6(10) != 45) return 18;        /* 0+..+9 = 45 < 50 */
    if (f6(100) != 55) return 19;       /* acc grows 0..9 -> 45, next +10 -> 55 > 50, exit */
    if (f7(3) != 3) return 20;
    if (f8(-1, 0) != 8) return 21;
    if (f8(-1, 5) != 7) return 22;
    return 0;
}
