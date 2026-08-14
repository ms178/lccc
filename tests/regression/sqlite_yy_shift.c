/* Regression (v4, sqlite3 3.53.4 parser crash): compare-replay must not
 * trust register assignments that were sized against the ORIGINAL Cmp
 * position. The replay re-emits the comparison at the consumer (a
 * non-adjacent Select / CondBranch), where the Cmp-lhs register may have
 * been reused by a later-defined value.
 *
 * This mirrors sqlite3's yy_shift tail exactly:
 *   stateno = state > 599 ? state+415 : state;   (state is U16)
 *   stack[i].stateno = stateno;
 *   stack[i].major   = major;
 *   memcpy(&stack[i].minor, &minor, 16);
 *
 * A wrong replay compares state+415 instead of state (the +415 add reuses
 * the Cmp-lhs register), corrupting the parser stack entry and crashing
 * sqlite3PExprIsNull later on garbage Expr pointers. We sweep all boundary
 * values around 599 and 1014 (=599+415) and verify the exact stateno.
 */
#include <stdio.h>
#include <string.h>

typedef unsigned short YYACTIONTYPE; /* == sqlite3 YYACTIONTYPE */
typedef unsigned short YYCODETYPE;
typedef struct { const char *z; unsigned int n; } Token; /* 16 bytes */

typedef struct yyStackEntry {
    YYACTIONTYPE stateno;
    YYCODETYPE major;
    Token minor;
} yyStackEntry;

#define YY_MAX_SHIFT 599
#define YY_MIN_REDUCE 1282
#define NSTK 64

static yyStackEntry stack[NSTK];

/* The yy_shift shape: one shift per call, state swept across boundaries.
 * Extra argument work keeps register pressure realistic so the allocator
 * may reuse the Cmp-lhs register before the Select. */
static YYACTIONTYPE yy_shift_like(YYACTIONTYPE state, YYCODETYPE major, Token minor, int idx) {
    yyStackEntry *yytos = &stack[idx];
    YYACTIONTYPE stateno;
    stateno = state > YY_MAX_SHIFT ? (YYACTIONTYPE)((int)state + 415) : state;
    yytos->stateno = stateno;
    yytos->major = major;
    memcpy(&yytos->minor, &minor, sizeof(Token));
    return stateno;
}

int main(void) {
    Token tok = { "abc", 3 };
    unsigned bad = 0;
    /* Sweep across both boundaries: 599 (max shift) and 1014 (599+415),
     * plus the U16 wrap-around edge. */
    static const unsigned vals[] = {
        0, 1, 598, 599, 600, 601, 1000, 1013, 1014, 1015,
        32767, 32768, 65534, 65535
    };
    for (unsigned i = 0; i < sizeof(vals) / sizeof(vals[0]); i++) {
        YYACTIONTYPE state = (YYACTIONTYPE)vals[i];
        YYACTIONTYPE want = state > 599 ? (YYACTIONTYPE)((int)state + 415) : state;
        YYACTIONTYPE got = yy_shift_like(state, 42, tok, (int)i);
        if (got != want) {
            printf("state=%u want=%u got=%u\n", state, want, got);
            bad++;
        }
        /* Verify the stack entry was written completely and correctly. */
        yyStackEntry *e = &stack[i];
        if (e->stateno != want || e->major != 42 || e->minor.z != tok.z || e->minor.n != 3) {
            printf("entry[%u] corrupt: stateno=%u major=%u z=%p n=%u\n",
                   i, e->stateno, e->major, (void *)e->minor.z, e->minor.n);
            bad++;
        }
    }
    /* Second pass reusing the same stack slots: the previous contents must
     * be fully overwritten (no stale minor bytes). */
    for (unsigned i = 0; i < 8; i++) {
        Token t = { 0, 0 };
        yy_shift_like(700, 7, t, (int)i);
        yyStackEntry *e = &stack[i];
        if (e->stateno != (700 > 599 ? (YYACTIONTYPE)(700 + 415) : 700)
            || e->major != 7 || e->minor.z != 0 || e->minor.n != 0) {
            printf("overwrite[%u] failed\n", i);
            bad++;
        }
    }
    if (bad) { printf("FAILED (%u)\n", bad); return 1; }
    printf("OK\n");
    return 0;
}
