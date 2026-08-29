/* Indirect calls whose target VALUE statically resolves to &symbol must be
 * emitted as DIRECT calls, never as `call *sym(%rip)`. The paravirt fast
 * path (pv_ops+off shape) resolves Load(GlobalAddr) — a DATA cell whose
 * content is the callee — and emits `call *sym+off(%rip)`. A func_ptr whose
 * own def chain bottoms out at GlobalAddr is the symbol's ADDRESS, not a
 * load from it; conflating the two dereferenced the function's first
 * instruction bytes as the call target (SIGSEGV; regression pin for
 * fnptr_param_assign and its 8 siblings in the suite).
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

/* Global function-pointer TABLE: the legitimate `call *sym(%rip)` shape
 * (GlobalCell). The call through table[1] must load the cell, not treat
 * &table as code. */
static int (*const table[2])(void *, PgHdr *) = { stress, stress };

int main(void) {
    struct PCache c = {0};
    set_stress(&c, stress, 0);
    if (!(c.xStress && c.xStress(0, 0) == 7))
        return 1;
    /* DirectGlobal shape through a local copy of the symbol address. */
    int (*f)(void *, PgHdr *) = stress;
    if (!f || f(0, 0) != 7)
        return 2;
    /* GlobalCell shape: real data cell. */
    if (table[1](0, 0) != 7)
        return 3;
    /* GEP-offset cell (paravirt-style): must load from table+8. The
     * char*-cast formulation of this shape trips a GCC 14.2 i386
     * miscompile (the devirtualized target is called through a register
     * that is never loaded), so the portable &table[1] form pins the same
     * load-from-cell-at-offset semantic for every target. */
    {
        int (** const pp)(void *, PgHdr *) = &table[1];
        if ((*pp)(0, 0) != 7)
            return 4;
    }
    return 0;
}
