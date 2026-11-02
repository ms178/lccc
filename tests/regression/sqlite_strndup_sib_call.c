/* Regression (sqlite3 3.53.4 CREATE TABLE SIGSEGV in vdbeChangeP4Full):
 *
 *   if (n == 0) n = strlen(zP4);          // diamond: multi-def Copy of I32 n
 *   p = malloc(n+1); memcpy(p, z, n); p[n] = 0;
 *
 * resolve_index peels the I32→I64 Cast, so folded SIB keys on the Copy dest.
 * That dest's last IR use is the Cast, BEFORE memcpy; the GEP sits AFTER.
 * Multi-def folded-index stretch historically merged required=[GEP, Store],
 * so memcpy was not call-spanning, Phase 2 homed n in %r10, lookaside/memcpy
 * clobbered it, and ensure_sib_index_form movslq'd the stale register for
 * p[n]=0 (vdbeChangeP4Full @ 0x4c0800, bad store 0x4c0b2f).
 *
 * Differential vs GCC. Extra live pointer params recreate register pressure
 * so n is a candidate for a caller-saved SIB index. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>

struct Lookaside {
    uint16_t sz;
    int nOut;
    char *a, *b, *c, *d, *e, *f;
};

__attribute__((noinline))
static char *dupn(struct Lookaside *db, const char *z, int n) {
    char *p;
    if (z == 0)
        return 0;
    if (n == 0)
        n = (int)strlen(z);
    size_t need = (size_t)n + 1;
    if (need <= db->sz && db->a) {
        p = db->a;
        db->a = *(char **)p;
        db->nOut++;
    } else if (need <= db->sz && db->b) {
        p = db->b;
        db->b = *(char **)p;
        db->nOut++;
    } else if (need <= db->sz && db->c) {
        p = db->c;
        db->c = *(char **)p;
        db->nOut++;
    } else if (need <= db->sz && db->d) {
        p = db->d;
        db->d = *(char **)p;
        db->nOut++;
    } else if (need <= db->sz && db->e) {
        p = db->e;
        db->e = *(char **)p;
        db->nOut++;
    } else if (need <= db->sz && db->f) {
        p = db->f;
        db->f = *(char **)p;
        db->nOut++;
    } else {
        p = malloc(need);
    }
    if (p) {
        memcpy(p, z, (size_t)n);
        p[n] = 0;
    }
    /* Keep lookaside pointers live across memcpy + p[n]=0 so the RA cannot
     * drop them before the indexed store (sqlite's sqlite3DbMallocRawNN). */
    if (db->a || db->b || db->c || db->d || db->e || db->f)
        db->nOut += 0;
    return p;
}

static int check(const char *z, int n) {
    struct Lookaside db = {0};
    db.sz = 8; /* force malloc path, matching a small lookaside */
    char *p = dupn(&db, z, n);
    if (!p)
        return 1;
    size_t expect = n == 0 ? strlen(z) : (size_t)n;
    int bad = p[expect] != 0 || memcmp(p, z, expect) != 0;
    /* terminator must not have landed at a stale (clobbered) index */
    if (expect > 0 && p[expect - 1] == 0 && z[expect - 1] != 0)
        bad = 1;
    free(p);
    return bad;
}

int main(void) {
    static const char *cases[] = {
        "hello-world-sqlite-p4",
        "CREATE TABLE t(a,b,c)",
        "x",
        "0123456789abcdef0123456789abcdef",
        "",
    };
    int fails = 0;
    for (size_t i = 0; i < sizeof(cases) / sizeof(cases[0]); i++) {
        int n = (int)strlen(cases[i]);
        fails += check(cases[i], n);
        fails += check(cases[i], 0); /* diamond n==0 → strlen */
    }
    if (fails) {
        fprintf(stderr, "FAIL %d\n", fails);
        return 1;
    }
    puts("OK");
    return 0;
}
