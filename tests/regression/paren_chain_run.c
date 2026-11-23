/* paren_chain_run.c -- peel semantics for pure paren chains `(`^k X `)`^k.
 *
 * The parser peels a pure parenthesized chain iteratively and parses the
 * content once (the limits-exprparen trampoline).  This fixture pins the
 * PEEL CONTRACT: bracket-balanced contents parse to the same value and type
 * in every operand position (primary, cast, sizeof, deref, call, index,
 * ternary, comma, operator mixtures), and non-chain groups (unbalanced
 * shapes, mixed-content forms) fall back to the ordinary recursive parser
 * with identical results.  chains_deep.c owns the 10k-depth scaling story.
 */
#include <stdio.h>

#define L1 (
#define R1 )
#define L2 L1 L1 L1 L1 L1 L1 L1 L1 L1 L1
#define R2 R1 R1 R1 R1 R1 R1 R1 R1 R1 R1

static int f3(int a, int b, int c) { return a * 100 + b * 10 + c; }
static int arr[4] = { 5, 6, 7, 8 };

#define CHECK(expr, want)                                                    \
    do {                                                                     \
        long got_ = (long)(expr);                                            \
        if (got_ != (long)(want)) {                                          \
            printf("FAIL %s:%d %s -> %ld != %ld\n", __FILE__, __LINE__,      \
                   #expr, got_, (long)(want));                               \
            fails++;                                                         \
        }                                                                    \
    } while (0)

int main(void)
{
    int fails = 0;
    int v = 7;
    int *p = &v;

    /* Primary position, all operator classes. */
    CHECK(L2 1 R2, 1);
    CHECK(-L2 1 R2, -1);
    CHECK(!L2 0 R2, 1);
    CHECK(~L2 0 R2, -1);
    CHECK(L2 2 R2 + L2 3 R2, 5);
    CHECK(L2 2 R2 * L2 3 R2 + L2 4 R2, 10);
    CHECK(L2 5 R2 > L2 4 R2, 1);

    /* Casts, deref, address-of, subscript. */
    CHECK(L2 (int)3 R2, 3);
    CHECK(L2 (long)2.5 R2, 2);
    CHECK(*L2 p R2, 7);
    CHECK(L2 &v R2 == p, 1);
    CHECK(L2 arr R2[1], 6);
    CHECK(arr[L2 2 R2], 7);
    CHECK(L2 arr R2[L2 3 R2], 8);

    /* Calls and comma/ternary contents. */
    CHECK(L2 f3 R2(1, 2, 3), 123);
    CHECK(f3((L2 1 R2), (L2 2 R2), L2 3 R2), 123);
    CHECK(L2 f3(1, 2, 3) R2, 123);
    CHECK((L2 1, 2, 3 R2), 3);
    CHECK(L2 0 ? 8 : 9 R2, 9);

    /* sizeof operand forms. */
    CHECK(sizeof L2 v R2, sizeof v);
    CHECK(sizeof(L2 v R2), sizeof v);
    CHECK(L2 sizeof v R2, sizeof v);

    /* Statement-expression tails are NOT chains: `({ ... })` stays on the
     * dedicated path even with extra parens around it. */
    CHECK(L2 ({ int t = 3; t + 1; }) R2, 4);
    CHECK(({ int t = 5; L2 t R2 * 2; }), 10);

    /* Non-chain fallbacks: mixed/unbalanced paren shapes must parse through
     * the ordinary recursive path with identical semantics. */
    CHECK((L2 v R2 + 1) * 2, 16);
    CHECK(((v)) + ((((1)))), 8);
    CHECK(f3((v), ((2)), (((3)))), 723);
    CHECK(((arr)[2]), 7);
    CHECK(*(&v), 7);
    CHECK(((int *)&v)[0], 7);

    if (fails == 0) {
        printf("ALL-OK\n");
    }
    return fails == 0 ? 0 : 1;
}
