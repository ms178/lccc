/* LICM must not speculate a faulting load into a non-dedicated preheader.
 *
 * SQLite 3.53.4 speedtest1 --testset json SIGSEGV (jsonCacheSearch inlined
 * into jsonParseFuncArg):
 *
 *   p = sqlite3_get_auxdata(ctx, JSON_CACHE_ID);
 *   if( p==0 ) return 0;
 *   for(i=0; i<p->nUsed; i++) ...
 *
 * The `if(p==0)` block is the loop's unique out-of-loop predecessor, so
 * find_preheader() returned it — but it is NOT dedicated (it also branches
 * to the early return). LICM hoisted the loop-invariant `p->nUsed` load to
 * the end of that block, dereferencing p+8 before the NULL test on every
 * call where the cache was absent.
 *
 * The fix: derived-pointer loads (call results, params, GEP chains) and
 * GEP-derived global loads require a *dedicated* preheader (unconditional
 * branch to the loop header). Dereferenceable classes (allocas, direct
 * data-symbol addresses) remain hoistable.
 *
 * This test drives the exact shape with a get_aux() that returns NULL, so
 * a regressing compiler segfaults deterministically. A second entry with a
 * non-NULL cache checks the loop still computes correct results.
 */
#include <stdio.h>
#include <string.h>

struct E {
    const char *zJson;
    int nJson;
};
struct Cache {
    void *db;
    int nUsed;
    struct E *a[4];
};

static struct Cache *g_cache;

/* Opaque through a volatile function pointer so the optimizer cannot
 * prove the return value non-NULL. */
static void *get_aux_impl(void *ctx, int id) {
    (void)ctx;
    (void)id;
    return g_cache;
}
static void *(*volatile get_aux)(void *, int) = get_aux_impl;

static struct E *search(void *ctx, const char *zJson, int nJson) {
    struct Cache *p;
    int i;
    p = get_aux(ctx, -429252);
    if (p == 0)
        return 0;
    for (i = 0; i < p->nUsed; i++) {
        if (p->a[i]->zJson == zJson)
            break;
    }
    if (i >= p->nUsed) {
        for (i = 0; i < p->nUsed; i++) {
            if (p->a[i]->nJson != nJson)
                continue;
            if (memcmp(p->a[i]->zJson, zJson, (unsigned)nJson) == 0)
                break;
        }
    }
    if (i < p->nUsed)
        return p->a[i];
    return 0;
}

int main(void) {
    static const char *k1 = "alpha";
    struct E e1 = { "alpha", 5 };
    struct E e2 = { "bravo", 5 };
    static struct Cache c;
    struct E *r;

    /* Phase 1: empty cache — the guarded path. A speculating compiler
     * dereferences NULL here. */
    g_cache = 0;
    r = search((void *)0x1, k1, 5);
    printf("empty=%p\n", (void *)r);

    /* Phase 2: populated cache — identity match then content match. */
    c.nUsed = 2;
    c.a[0] = &e1;
    c.a[1] = &e2;
    g_cache = &c;
    r = search((void *)0x1, e1.zJson, 5);
    printf("ident=%s\n", r ? r->zJson : "(nil)");
    r = search((void *)0x1, "bravo", 5); /* different pointer, same bytes */
    printf("bytes=%s\n", r ? r->zJson : "(nil)");
    return !(r && strcmp(r->zJson, "bravo") == 0);
}
