/* Function-pointer parameters must keep function-pointer type in sema.
 *
 * SQLite 3.53.4 amalgamation failed with:
 *   p->xStress = xStress;
 *   incompatible pointer types (have 'int *' but expected
 *   'int (*)(void *, struct PgHdr *)')
 *
 * The parser stores `int (*cb)(void *, T *)` as type_spec=int plus
 * fptr_params. Sema used only type_spec, so the parameter was `int *`.
 */
typedef struct PgHdr PgHdr;
struct PCache {
    int (*xStress)(void *, PgHdr *);
    void *pStress;
};

static int stress(void *ctx, PgHdr *pg) {
    (void)ctx;
    (void)pg;
    return 7;
}

static void set_stress(struct PCache *p, int (*xStress)(void *, PgHdr *), void *ctx) {
    p->xStress = xStress;
    p->pStress = ctx;
}

int main(void) {
    struct PCache c = {0};
    set_stress(&c, stress, 0);
    return (c.xStress && c.xStress(0, 0) == 7) ? 0 : 1;
}
