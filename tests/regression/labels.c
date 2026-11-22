/* labels.c -- flat label-run torture: every label-run form in one TU.
 *
 * A `Labeled(LabelRun)` binds an arbitrary run of `name:`/`case`/
 * `case lo ... hi:`/`default` labels to ONE body statement.  This fixture
 * exercises every run shape GCC 16.2 accepts: dense and sparse case runs,
 * ranges, `default` anywhere in the run, empty-statement tails (`case 0: :`
 * chains -- the -O0 duplicated-terminator class), statement-expression
 * tails, GNU label attributes, goto interactions, and the C2y named-loop
 * binding rules (trailing names bind `break`/`continue`; a `case` between
 * name and loop severs the binding).  Byte-identical to GCC on both arches.
 */
#include <stdio.h>

#define R10(b) \
    case (b): case (b)+1: case (b)+2: case (b)+3: case (b)+4: \
    case (b)+5: case (b)+6: case (b)+7: case (b)+8: case (b)+9:

static int dense(int k)
{
    switch (k) {
    R10(0)
    R10(10)
    R10(20)
        return 1;
    default:
        return 0;
    }
}

static int sparse_and_range(int k)
{
    int acc = 0;
    switch (k) {
    case 0: case 5: case 100: case -7:
        acc += 1;
        /* fall through */
    case 3 ... 4:
        acc += 10;
        break;
    case 50 ... 50:
        acc += 100;
        break;
    default:
        acc += 1000;
        break;
    }
    return acc;
}

static int default_positions(int k)
{
    switch (k) {
    default: case 1:
        return 1;
    }
}

static int default_middle(int k)
{
    switch (k) {
    case 1: default: case 2:
        return 2;
    }
}

/* Empty-statement tails: runs whose body is the empty statement `;`.
 * (A bare `:` is not a statement -- the -O0 duplicated-terminator bug fired
 * on these empty-statement tails pre-fix.) */
static int empty_tail_chains(int k)
{
    int acc = 0;
    switch (k) {
    case 0: case 1: ;
        acc += 1;
        break;
    case 2: case 3: ;
        acc += 2;
        break;
    case 4: ;
        acc += 3;
        break;
    e1: e2: case 5: ;
        acc += 9;
        break;
    case 6:
    case 7:
        break;
    case 8: {
        acc += 4;
        break;
    }
    default:
        acc -= 1;
        break;
    }
    return acc;
}

/* Statement-expression tail: the run's body is a GNU statement expression
 * that mutates a local, then the run falls out of the switch. */
static int stmt_expr_tail(int k)
{
    int ssum = 0;
    switch (k) {
    case 1: case 2: ({
        ssum += k * 10 + 1;
    });
        break;
    case 3:
        return ({ int t = 3; t * 3; });
    default:
        return -1;
    }
    return ssum;
}

/* Name labels in runs, goto interactions, GNU label attributes. */
static int names_and_gotos(int k)
{
    int acc = 0;
    if (k == 1)
        goto mid;
    if (k == 2)
        goto tail;
start: mid: tail:
    acc += 1;
    {
        lbl2: __attribute__((unused)) int z = 1;
        (void)z;
        acc += 2;
    }
    return acc;
}

/* C2y named loops: trailing names bind break/continue; a case between the
 * name and the loop severs the binding (labels.c binding rules). */
static int named_loops(int mode)
{
    int acc = 0;
    int i;
outer:
    for (i = 0; i < 5; i++) {
        int j;
    inner:
        for (j = 0; j < 5; j++) {
            if (mode == 1 && i == 2)
                continue outer;
            if (mode == 2 && j == 2)
                break inner;
            if (mode == 3 && i + j == 3)
                break outer;
            acc += 1;
        }
        if (mode == 4 && i == 1)
            break outer;
    }
    /* Name label directly wrapping a while loop. */
    if (mode == 5) {
        int n = 0;
    wl: while (n < 3) {
            n++;
            if (n == 2)
                continue wl;
            acc += 10;
        }
    }
    /* Label-run names before a loop all bind (trailing names). */
    if (mode == 6) {
        int n = 0;
    a6: b6: while (n < 2) {
            n++;
            if (n == 1)
                continue b6;
            acc += 100;
        }
    }
    return acc;
}

/* Nested switch with runs in both arms and fallthrough across runs. */
static int nested(int k)
{
    int acc = 0;
    switch (k) {
    case 0: case 1:
        switch (k * 2) {
        case 0: case 2:
            acc += 5;
            /* fall through */
        default: case 3:
            acc += 6;
            break;
        }
        /* fall through */
    case 2:
        acc += 7;
        break;
    default:
        acc += 8;
        break;
    }
    return acc;
}

int main(void)
{
    int fails = 0;
    int k;
    long sum;

    for (k = -1; k <= 32; k++) {
        if (dense(k) != (k >= 0 && k < 30)) {
            printf("FAIL dense %d\n", k);
            fails++;
        }
    }

    if (sparse_and_range(0) != 11 || sparse_and_range(3) != 10 ||
        sparse_and_range(50) != 100 || sparse_and_range(5) != 11 ||
        sparse_and_range(9) != 1000) {
        printf("FAIL sparse/range\n");
        fails++;
    }
    if (default_positions(7) != 1 || default_positions(1) != 1 ||
        default_middle(3) != 2 || default_middle(2) != 2) {
        printf("FAIL default positions\n");
        fails++;
    }
    if (empty_tail_chains(0) != 1 || empty_tail_chains(2) != 2 ||
        empty_tail_chains(4) != 3 || empty_tail_chains(5) != 9 ||
        empty_tail_chains(7) != 0 || empty_tail_chains(8) != 4 ||
        empty_tail_chains(9) != -1) {
        printf("FAIL empty tail chains\n");
        fails++;
    }
    if (stmt_expr_tail(1) != 11 || stmt_expr_tail(2) != 21 ||
        stmt_expr_tail(3) != 9 || stmt_expr_tail(0) != -1) {
        printf("FAIL stmt-expr tail\n");
        fails++;
    }
    if (names_and_gotos(1) != 3 || names_and_gotos(2) != 3 ||
        names_and_gotos(0) != 3 || names_and_gotos(-1) != 3) {
        printf("FAIL names/gotos\n");
        fails++;
    }
    if (named_loops(1) != 20 || named_loops(2) != 10 ||
        named_loops(3) != 3 || named_loops(4) != 10 ||
        named_loops(5) != 45 || named_loops(6) != 125 ||
        named_loops(0) != 25) {
        printf("FAIL named loops: %d %d %d %d %d %d %d\n", named_loops(0),
               named_loops(1), named_loops(2), named_loops(3),
               named_loops(4), named_loops(5), named_loops(6));
        fails++;
    }
    if (nested(0) != 18 || nested(1) != 18 || nested(2) != 7 ||
        nested(3) != 8) {
        printf("FAIL nested\n");
        fails++;
    }

    /* Aggregate over the dense run: the whole decade battery at once. */
    sum = 0;
    for (k = 0; k < 30; k++) {
        sum += dense(k);
    }
    if (sum != 30) {
        printf("FAIL dense sum %ld\n", sum);
        fails++;
    }

    if (fails == 0) {
        printf("ALL-OK\n");
    }
    return fails == 0 ? 0 : 1;
}
