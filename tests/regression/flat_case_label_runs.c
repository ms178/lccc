/* flat_case_label_runs.c -- flat case/label-run semantics matrix.
 *
 * Exercises the flat Stmt::Labeled(LabelRun) lowering on every runtime
 * semantic the rewrite must preserve:
 *   - dense case runs (R10/R100 macros, which already end in ':' -- never
 *     write `R10 :`, that is a stray colon), sharing one body;
 *   - sparse / negative / mixed case runs with fallthrough across runs;
 *   - default before, between and after case runs;
 *   - goto into a named label sitting inside a case run (bounded, no loop);
 *   - label runs (a: b: c: stmt) reached by goto at each entry point;
 *   - labeled loops with goto/break/continue;
 *   - nested switches, switch in loops, runs around compound statements.
 *
 * Output must be byte-identical to GCC 16.2 on x86-64 and i686.
 */
#include <stdio.h>

/* Case runs: R10(b) covers b..b+9; R100 = cases 0..99 via ten decades.
 * The macro already ends in ':' -- never write `R10(0) :`. */
#define R10(b) \
    case (b): case (b)+1: case (b)+2: case (b)+3: case (b)+4: \
    case (b)+5: case (b)+6: case (b)+7: case (b)+8: case (b)+9:
#define R100 R10(0) R10(10) R10(20) R10(30) R10(40) \
             R10(50) R10(60) R10(70) R10(80) R10(90)

/* Dense run: cases 0..99 share one body. */
static int f_dense(int v)
{
    switch (v) {
    R100
        return 1;
    default:
        return 0;
    }
}

/* Mixed run: sparse cases, a named label inside the switch, and a goto
 * into it.  The goto is bounded (no loop anywhere in this function). */
static int f_mixed(int v)
{
    int acc = 0;
    switch (v) {
    case 1000:
    case 1001:
        acc = 1;
        goto nm;
    case 2000:
        acc = 2;
        break;
    nm:
    case 3000:
    case 3001:
        acc += 10;
        break;
    default:
        acc = -1;
        break;
    }
    return acc;
}

/* Fallthrough across two adjacent runs. */
static int f_fall(int v)
{
    int acc = 0;
    switch (v) {
    case 0:
    case 1:
        acc = 1;
        /* fall through */
    case 2:
    case 3:
        acc += 2;
        break;
    case 4:
        acc = 9;
        break;
    }
    return acc;
}

/* Default first, then a run, then default-position run tail. */
static int f_default_first(int v)
{
    switch (v) {
    default:
        return -1;
    case 50:
    case 51:
        return 5;
    }
}

/* Negative and very large case labels interleaved with a run. */
static int f_sparse(int v)
{
    switch (v) {
    case -3:
    case -2:
    case -1:
        return 11;
    case 1 << 30:
        return 12;
    case 7:
        return 13;
    }
    return 0;
}

/* Case runs inside a loop: break/continue interact with the run bodies. */
static int f_loop(int n)
{
    int acc = 0;
    for (int i = 0; i < n; i++) {
        switch (i % 4) {
        case 0:
        case 1:
            acc += 1;
            continue;
        case 2:
            acc += 10;
            break;
        case 3:
            acc += 100;
            break;
        }
        acc += 1000;
    }
    return acc;
}

/* Label run (x: y: z:) plus per-label entry points with goto. */
static int g_labels(int v)
{
    int acc = 0;
    if (v == 1)
        goto c;
    if (v == 2)
        goto b;
    if (v == 3)
        goto a;
    if (v == 4)
        return -1;
    a:
        acc += 1;
    b:
        acc += 2;
    c:
        acc += 4;
    x:
    y:
    z:
        acc += 100;
    return acc;
}

/* Labeled loop: goto restarts, break leaves, continue advances. */
static int g_loop_labels(int n)
{
    int acc = 0, i = 0;
    restart:
    while (i < n) {
        if (i == 3) {
            i++;
            goto restart; /* bounded: i strictly advances */
        }
        if (i == 5) {
            i += 2;
            continue;
        }
        if (i >= 8)
            break;
        acc += i;
        i++;
    }
    return acc;
}

/* Nested switches: run in outer, run in inner, break targets. */
static int g_nested(int v)
{
    int acc = 0;
    switch (v) {
    case 1:
    case 2:
        switch (v * 10) {
        case 10:
        case 20:
            acc = 100;
            break;
        default:
            acc = 200;
            break;
        }
        acc += 1;
        break;
    case 3:
        acc = 3;
        break;
    }
    return acc;
}

static int g_case_block(int v)
{
    switch (v) {
    case 6:
    case 7: {
        int t = v * 2;
        if (t > 12)
            return t;
        return t + 1;
    }
    case 8:
        return 8;
    }
    return 0;
}

#define CHECK(expr, want)                                              \
    do {                                                               \
        int got_ = (expr);                                             \
        if (got_ != (want)) {                                          \
            printf("FAIL %s: got %d want %d\n", #expr, got_, (want));   \
            fails++;                                                   \
        }                                                              \
    } while (0)

int main(void)
{
    int fails = 0;

    CHECK(f_dense(0), 1);
    CHECK(f_dense(55), 1);
    CHECK(f_dense(99), 1);
    CHECK(f_dense(100), 0);
    CHECK(f_dense(-1), 0);

    CHECK(f_mixed(1000), 11);
    CHECK(f_mixed(1001), 11);
    CHECK(f_mixed(2000), 2);
    CHECK(f_mixed(3000), 10);
    CHECK(f_mixed(3001), 10);
    CHECK(f_mixed(7), -1);

    CHECK(f_fall(0), 3);
    CHECK(f_fall(1), 3);
    CHECK(f_fall(2), 2);
    CHECK(f_fall(3), 2);
    CHECK(f_fall(4), 9);
    CHECK(f_fall(9), 0);

    CHECK(f_default_first(50), 5);
    CHECK(f_default_first(51), 5);
    CHECK(f_default_first(52), -1);

    CHECK(f_sparse(-3), 11);
    CHECK(f_sparse(-1), 11);
    CHECK(f_sparse(1 << 30), 12);
    CHECK(f_sparse(7), 13);
    CHECK(f_sparse(8), 0);

    CHECK(f_loop(0), 0);
    CHECK(f_loop(1), 1);
    CHECK(f_loop(2), 2);
    CHECK(f_loop(3), 1012);
    CHECK(f_loop(4), 2112);
    CHECK(f_loop(9), 4225);

    CHECK(g_labels(0), 107);
    CHECK(g_labels(1), 104);
    CHECK(g_labels(2), 106);
    CHECK(g_labels(3), 107);
    CHECK(g_labels(4), -1);

    CHECK(g_loop_labels(0), 0);
    CHECK(g_loop_labels(4), 3);
    CHECK(g_loop_labels(9), 14);
    CHECK(g_loop_labels(20), 14);

    CHECK(g_nested(1), 101);
    CHECK(g_nested(2), 101);
    CHECK(g_nested(3), 3);
    CHECK(g_nested(4), 0);

    CHECK(g_case_block(6), 13);
    CHECK(g_case_block(7), 14);
    CHECK(g_case_block(8), 8);
    CHECK(g_case_block(9), 0);

    if (fails == 0)
        printf("ALL-OK\n");
    return fails != 0;
}
