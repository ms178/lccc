/* paren_chain_deep.c -- deep parenthesized-expression chains.
 *
 * The LBR/RBR X-macro ladder builds `(`^10000 and `)`^10000 (LBR4 0 RBR4 =
 * 10,000 parens around one token, the gcc.c-torture/compile/limits-exprparen
 * shape).  `paren_chain_run` peels such pure chains iteratively, so they must
 * parse, fold and run at any depth.  The 19 walk-throughs exercise the peel's
 * content classification (X non-empty, bracket-balanced) across every primary
 * and postfix shape, including the unbalanced forms that must fall back to
 * the ordinary recursive parser and still be right.
 *
 * Output must be byte-identical to GCC 16.2 on x86-64 and i686.
 */
#include <stdio.h>

#define LBR1 (
#define RBR1 )
#define LBR2 LBR1 LBR1 LBR1 LBR1 LBR1 LBR1 LBR1 LBR1 LBR1 LBR1
#define RBR2 RBR1 RBR1 RBR1 RBR1 RBR1 RBR1 RBR1 RBR1 RBR1 RBR1
#define LBR3 LBR2 LBR2 LBR2 LBR2 LBR2 LBR2 LBR2 LBR2 LBR2 LBR2
#define RBR3 RBR2 RBR2 RBR2 RBR2 RBR2 RBR2 RBR2 RBR2 RBR2 RBR2
#define LBR4 LBR3 LBR3 LBR3 LBR3 LBR3 LBR3 LBR3 LBR3 LBR3 LBR3
#define RBR4 RBR3 RBR3 RBR3 RBR3 RBR3 RBR3 RBR3 RBR3 RBR3 RBR3

/* 10k-deep pure chains at static-initializer depth and run time. */
static int z0 = LBR4 0 RBR4;
static int z1 = LBR4 7 RBR4 + LBR3 3 RBR3;

struct s { int f; };
static int (*fp)(int);

static int g(int x) { return x + 1; }

#define CHECK(expr, want)                                              \
    do {                                                               \
        long got_ = (long)(expr);                                      \
        if (got_ != (long)(want)) {                                    \
            printf("FAIL line %d: got %ld want %ld\n", __LINE__, got_, \
                   (long)(want));                                      \
            fails++;                                                   \
        }                                                              \
    } while (0)

int main(void)
{
    int fails = 0;
    int v = 7, c = 1, a = 5, b = 9, x = 0;
    int arr[4] = { 10, 11, 12, 13 };
    struct s ss = { 42 };
    int q[2] = { LBR4 1 RBR4, LBR4 2 RBR4 };
    fp = g;

    CHECK(z0, 0);
    CHECK(z1, 10);
    CHECK(LBR4 0 RBR4, 0);

    /* 1. paren-assign */
    x = LBR4 7 RBR4;
    CHECK(x, 7);

    /* 2. paren-call */
    CHECK(g(LBR4 3 RBR4), 4);

    /* 3. GNU ?: under deep parens */
    CHECK(LBR4 c ? a : b RBR4, 5);
    CHECK(LBR4 !c ? a : b RBR4, 9);

    /* 4. cast as peel content */
    CHECK(LBR4 (int)LBR1 2.0 RBR1 RBR4, 2);

    /* 5. statement expression as peel content */
    CHECK(LBR4 ({ 41 + 1; }) RBR4, 42);

    /* 6. comma as peel content */
    CHECK(LBR4 (1, 2) RBR4, 2);

    /* 7. compound-assign */
    x = 100;
    x += LBR4 3 RBR4;
    CHECK(x, 103);
    x *= LBR4 2 RBR4;
    CHECK(x, 206);

    /* 8. subscript */
    CHECK(arr[LBR4 1 RBR4], 11);

    /* 9. member access through peeled parens */
    CHECK(LBR4 ss RBR4.f, 42);
    CHECK((LBR4 ss RBR4).f, 42);

    /* 10. dereference of peeled parens */
    {
        int *p = &v;
        CHECK(*LBR4 p RBR4, 7);
    }

    /* 11. address-of peeled parens */
    CHECK(*&LBR4 v RBR4, 7);

    /* 12. unary operators outside the chain */
    CHECK(-LBR4 1 RBR4, -1);
    CHECK(!LBR4 0 RBR4, 1);
    CHECK(~LBR4 0 RBR4, -1);

    /* 13. sizeof with peeled parens (expression operand; a chain around a
     * TYPE name is not valid C -- `((int))` re-parses as parenthesized
     * expressions with a missing cast operand -- so the cast walk-through
     * above is the sanctioned type-path coverage). */
    CHECK(sizeof LBR4 v RBR4, sizeof v);
    CHECK(sizeof(LBR4 v RBR4), sizeof v);

    /* 14. binary mix of two chains */
    CHECK(LBR4 2 RBR4 * LBR4 3 RBR4, 6);
    CHECK(LBR2 1 RBR2 + LBR3 2 RBR3, 3);

    /* 15. condition position */
    if (LBR4 1 RBR4)
        CHECK(1, 1);
    else
        CHECK(0, 1);

    /* 16. ternary chain nesting */
    CHECK(LBR4 LBR4 c RBR4 ? 1 : 2 RBR4, 1);

    /* 17. aggregate initializers */
    CHECK(q[0], 1);
    CHECK(q[1], 2);

    /* 18. return-position shape (via helper) */
    CHECK(g(LBR4 41 RBR4), 42);

    /* 19. function-pointer call through peeled parens */
    CHECK(LBR4 fp RBR4(LBR4 9 RBR4), 10);

    /* Unbalanced contents fall back to the recursive parser and must still
     * be right: `(((v)+(a)))`, `(((int)3))`, chains inside subscript
     * arithmetic. */
    CHECK((LBR1 (v) + (a) RBR1), 12);
    CHECK(LBR1 LBR1 (int)3 RBR1 RBR1, 3);
    CHECK(arr[LBR2 0 RBR2 + 1], 11);

    /* Deep chains at every operand position of one expression. */
    CHECK(LBR4 1 RBR4 + LBR4 2 RBR4 * LBR4 3 RBR4 - LBR4 4 RBR4, 3);

    if (fails == 0)
        printf("ALL-OK\n");
    return fails != 0;
}
