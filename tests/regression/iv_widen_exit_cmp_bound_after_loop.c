/* iv_widen: a loop-EXIT compare whose other operand is defined AFTER the loop
 * must not get its widening cast hoisted into the preheader.
 *
 * SQLite 3.53.4 selectExpander():
 *     for(k=0; k<pEList->nExpr; k++){ ... if( star ) break; ... }
 *     if( k<pEList->nExpr ){ ...expand "*"... }
 * reloads pEList->nExpr in the loop exit block. iv_widen widened `k` to I64
 * (it indexes pEList->a[k]) and rewrote the exit compare to I64 with
 * `sext(nExpr_reload)` appended to the PREHEADER -- a use before def. The
 * backend read whatever the register held (movslq %edi,%r13 of the incoming
 * pointer), so `SELECT * FROM (SELECT 1 AS a, 2 AS b)` intermittently
 * (ASLR-dependent) skipped the star expansion and printed "column1"/NULL,
 * which corrupted schema reparse and ended in a SIGSEGV. The IR verifier
 * (CCC_VERIFY_IR, armed by run_regression_suite.sh) flags the def-dominates-
 * use violation; the output checks below catch the wrong-value symptom.
 *
 * Variants: signed IV with a reloaded bound (the SQLite shape), a bound that
 * is a post-loop call result, an unsigned counted IV, and the control where
 * the bound is loop-invariant and available in the preheader (must still be
 * widened and correct). */
#include <stdio.h>

struct Expr {
    unsigned char op;
    unsigned flags;
    struct Expr *pRight;
};
struct Item {
    struct Expr *pExpr;
    char *zName;
    long pad;
};
struct List {
    int nExpr;
    struct Item a[4];
};

#define TK_ASTERISK 180
#define TK_DOT 142

__attribute__((noinline)) int find_star(struct List *p, unsigned *flagsOut)
{
    int k;
    unsigned fl = 0;
    for (k = 0; k < p->nExpr; k++) {
        struct Expr *e = p->a[k].pExpr;
        if (e->op == TK_ASTERISK)
            break;
        if (e->op == TK_DOT && e->pRight->op == TK_ASTERISK)
            break;
        fl |= e->flags;
    }
    *flagsOut = fl;
    if (k < p->nExpr) /* bound reloaded in the exit block */
        return k;
    return -1;
}

static int limit_calls;
__attribute__((noinline)) int get_limit(struct List *p)
{
    limit_calls++;
    return p->nExpr;
}

__attribute__((noinline)) int find_zero_then_call(struct List *p, int n)
{
    int k;
    for (k = 0; k < n; k++)
        if (p->a[k].pad == 0)
            break;
    if (k < get_limit(p)) /* bound is a post-loop call result */
        return k;
    return -1;
}

__attribute__((noinline)) int ufind(const unsigned char *s, unsigned n, const unsigned *lim)
{
    unsigned i;
    for (i = 0; i < n; i++)
        if (s[i] == 'x')
            break;
    if (i < *lim) /* unsigned IV, bound loaded after the loop */
        return (int)i;
    return -1;
}

__attribute__((noinline)) int find_star_invariant(struct List *p)
{
    int n = p->nExpr; /* available in the preheader: stays widened */
    int k;
    for (k = 0; k < n; k++)
        if (p->a[k].pExpr->op == TK_ASTERISK)
            break;
    if (k < n)
        return k;
    return -1;
}

int main(void)
{
    struct Expr star = {TK_ASTERISK, 0, 0}, x = {1, 4, 0}, y = {2, 8, 0};
    struct Expr dotstar = {TK_DOT, 16, &star};
    struct List l = {2, {{&x, 0, 1}, {&star, 0, 0}}};
    struct List m = {2, {{&x, 0, 1}, {&y, 0, 2}}};
    struct List d = {3, {{&y, 0, 1}, {&x, 0, 1}, {&dotstar, 0, 1}}};
    unsigned f1, f2, f3;
    int fails = 0;

    int a = find_star(&l, &f1);
    int b = find_star(&m, &f2);
    int c = find_star(&d, &f3);
    if (a != 1 || f1 != 4 || b != -1 || f2 != 12 || c != 2 || f3 != 12) {
        printf("find_star: %d %u %d %u %d %u\n", a, f1, b, f2, c, f3);
        fails++;
    }

    int z1 = find_zero_then_call(&l, 2); /* a[1].pad == 0 -> 1 */
    int z2 = find_zero_then_call(&m, 2); /* none -> k=2, 2<2 false -> -1 */
    if (z1 != 1 || z2 != -1 || limit_calls != 2) {
        printf("find_zero_then_call: %d %d calls=%d\n", z1, z2, limit_calls);
        fails++;
    }

    unsigned lim3 = 3, lim9 = 9;
    int u1 = ufind((const unsigned char *)"abxd", 4, &lim3);
    int u2 = ufind((const unsigned char *)"abcd", 4, &lim9);
    int u3 = ufind((const unsigned char *)"abcx", 4, &lim3);
    if (u1 != 2 || u2 != 4 || u3 != -1) {
        printf("ufind: %d %d %d\n", u1, u2, u3);
        fails++;
    }

    int i1 = find_star_invariant(&l), i2 = find_star_invariant(&m);
    if (i1 != 1 || i2 != -1) {
        printf("find_star_invariant: %d %d\n", i1, i2);
        fails++;
    }

    printf("%s\n", fails ? "FAIL" : "OK");
    return fails != 0;
}
