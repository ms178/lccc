/* Bool-phi merge-diamond branch threading: value-context short-circuits,
 * explicit if/else bool diamonds, second-phi merges, partial threading
 * (critical-edge preds), and loop-carried self-referential bools must all
 * preserve exact C semantics at every tier. Each pattern is checked against
 * an independently computed expectation over the full small input space. */
#include <stdio.h>

static volatile int v_in[6] = {0, 1, 2, 3, 5, -1};

/* P1: && value-context, both arms constant. */
static int p1(int a, int b) { int ok = a && b; if (ok) return 11; return 22; }

/* P2: || value-context. */
static int p2(int a, int b) { int ok = a || b; if (ok) return 33; return 44; }

/* P3: mixed arm — one constant, one computed. */
static int p3(int a, int b) {
    int ok;
    if (a) ok = 1; else ok = (b > 3);
    if (ok) return 55;
    return 66;
}

/* P4: three predecessors into the same bool merge. */
static int p4(int a, int b, int c) {
    int ok;
    if (a) ok = 1;
    else if (b) ok = (c != 0);
    else ok = 0;
    if (ok) return 77;
    return 88;
}

/* P5: bool with an extra arithmetic use — threading is partial, the phi
 * must survive for the add while the branch is threaded. */
static int g_side(int x) { return x + 1; }
static int p5(int a, int b) {
    int ok = a && b;
    int r = ok + g_side(1);
    if (ok) return r;
    return 99;
}

/* P6: second (non-bool) phi merged on the same edge, consumed only in the
 * branch targets. */
static int p6(int a, int b) {
    int v, ok;
    if (a) { v = 1; ok = 1; }
    else   { v = 2; ok = (b != 0); }
    if (ok) return v + 100;
    return v + 200;
}

/* P7: unthreadable critical-edge predecessor (CondBranch into the merge)
 * mixed with threadable ones — partial threading must stay sound. */
static int p7(int a, int b, int c) {
    int ok = 0;
    if (a) ok = 1;
    if (b) ok = (c > 1);
    else   ok = 0;
    if (ok) return 111;
    return 222;
}

/* P8: loop-carried self-referential bool — the latch pred must be skipped. */
static int p8(int n, int limit) {
    int ok = 0;
    for (int i = 0; i < n; i++) {
        if (i > limit) ok = 1;
        else           ok = ok;
    }
    if (ok) return 333;
    return 444;
}

/* P9: nested diamonds — fixpoint iteration collapses both levels. */
static int p9(int a, int b, int c) {
    int ok = a && b;
    if (ok) {
        int ok2 = c || a;
        if (ok2) return 555;
        return 556;
    }
    return 557;
}

/* P10: _Bool type identity for the threaded branch. */
static _Bool p10(int a, int b) {
    _Bool ok = (_Bool)a && (_Bool)b;
    if (ok) return 1;
    return 0;
}

/* P11: bool merged from compare arms of differing widths, then branched
 * twice (second branch reads the same phi). */
static int p11(int a, char b) {
    int ok;
    if (a) ok = (b < 'x');
    else   ok = 0;
    if (ok) return 1;
    if (ok) return 2; /* dead but keeps ok multi-use */
    return 3;
}

int main(void) {
    int fails = 0;
    for (int i = 0; i < 6; i++)
        for (int j = 0; j < 6; j++)
            for (int k = 0; k < 6; k++) {
                int a = v_in[i], b = v_in[j], c = v_in[k];
                if (p1(a, b) != (a && b ? 11 : 22)) { printf("P1 %d %d\n", a, b); fails++; }
                if (p2(a, b) != (a || b ? 33 : 44)) { printf("P2 %d %d\n", a, b); fails++; }
                if (p3(a, b) != (a ? 55 : (b > 3 ? 55 : 66))) { printf("P3 %d %d\n", a, b); fails++; }
                if (p4(a, b, c) != ((a || (b && c != 0)) ? 77 : 88)) { printf("P4 %d %d %d\n", a, b, c); fails++; }
                {
                    int ok = a && b;
                    int exp = ok ? ok + 2 : 99;
                    if (p5(a, b) != exp) { printf("P5 %d %d\n", a, b); fails++; }
                }
                {
                    int v, ok;
                    if (a) { v = 1; ok = 1; } else { v = 2; ok = (b != 0); }
                    if (p6(a, b) != (ok ? v + 100 : v + 200)) { printf("P6 %d %d\n", a, b); fails++; }
                }
                {
                    int ok = 0;
                    if (a) ok = 1;
                    if (b) ok = (c > 1); else ok = 0;
                    if (p7(a, b, c) != (ok ? 111 : 222)) { printf("P7 %d %d %d\n", a, b, c); fails++; }
                }
                {
                    int ok = 0;
                    for (int t = 0; t < a; t++) { if (t > b) ok = 1; }
                    if (p8(a, b) != (ok ? 333 : 444)) { printf("P8 %d %d\n", a, b); fails++; }
                }
                {
                    int ok = a && b;
                    int exp;
                    if (ok) { int ok2 = c || a; exp = ok2 ? 555 : 556; } else exp = 557;
                    if (p9(a, b, c) != exp) { printf("P9 %d %d %d\n", a, b, c); fails++; }
                }
                if (p10(a, b) != ((_Bool)a && (_Bool)b)) { printf("P10 %d %d\n", a, b); fails++; }
                {
                    int ok = a ? ((char)b < 'x') : 0;
                    if (p11(a, (char)b) != (ok ? 1 : 3)) { printf("P11 %d %d\n", a, b); fails++; }
                }
            }
    if (fails) { printf("%d FAILURES\n", fails); return 1; }
    puts("bool-thread patterns PASS");
    return 0;
}
