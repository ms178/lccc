/* Regression (v4, sqlite3 3.53.4 VDBE corruption): the LEA→SIB peephole
 * folded `leaq (base,index),%r13; movq %r13,%rcx; movX ...,(%rcx)` into
 * `movX ...,(base,index)` while %r13 was still READ by the sibling field
 * stores that follow (aOp[i].p1/p2/p3 via the const-offset GEP fold).
 * The fold removed the only definition of %r13, so those stores wrote
 * through a never-defined register (0 / stale) — corrupting the Vdbe op
 * array and crashing later in sqlite3OpenSchemaTable.
 *
 * This mirrors the sqlite3 add-op fast path: a variable-offset GEP
 * (&aOp[i]) whose result is shared by the opcode store AND the
 * p1/p2/p3/p4type/p5 field stores, with enough surrounding pressure so
 * the GEP result lives in a callee-saved register. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef struct VdbeOp {
    unsigned char opcode;   /* +0 */
    signed char p4type;     /* +1 */
    unsigned short p1;      /* +2 */
    unsigned short p2;      /* +4 */
    unsigned short p3;      /* +6 */
    unsigned char p5;       /* +8 */
    long long p4;           /* +16 */
} VdbeOp; /* 24 bytes */

typedef struct Vdbe {
    char pad[0x88];         /* aOp @ 0x88, nOp @ 0x90, nOpAlloc @ 0x94 */
    VdbeOp *aOp;
    int nOp;
    int nOpAlloc;
} Vdbe;

static Vdbe *vdbes[6]; /* extra pressure: several Vdbes live at once */

static void addop(Vdbe *v, unsigned char op, int p1, int p2, int p3) {
    int i = v->nOp++;
    v->aOp[i].opcode = op;
    v->aOp[i].p4type = 0;
    v->aOp[i].p1 = p1;
    v->aOp[i].p2 = p2;
    v->aOp[i].p3 = p3;
    v->aOp[i].p5 = 0;
    v->aOp[i].p4 = 0;
}

static void run(Vdbe *v, int base) {
    addop(v, 0x74, 0, base, 5);      /* OP_OpenWrite */
    addop(v, 0x08, base, 0, 0);      /* OP_If */
    addop(v, 0x81, 1, base + 1, 0);  /* OP_SetCookie-ish */
    addop(v, 0x95, base, 2, 1);      /* OP_CreateBtree-ish */
}

int main(void) {
    int bad = 0;
    for (int k = 0; k < 6; k++) {
        Vdbe *v = calloc(1, sizeof(Vdbe));
        v->aOp = calloc(64, sizeof(VdbeOp));
        vdbes[k] = v;
        run(v, k * 100);
    }
    /* Verify every appended op: opcode + all fields must be intact. */
    static const unsigned char want_op[4] = {0x74, 0x08, 0x81, 0x95};
    for (int k = 0; k < 6; k++) {
        Vdbe *v = vdbes[k];
        if (v->nOp != 4) { printf("v[%d] nOp=%d want 4\n", k, v->nOp); bad++; }
        for (int i = 0; i < 4; i++) {
            VdbeOp *op = &v->aOp[i];
            int ok = op->opcode == want_op[i]
                && op->p4type == 0
                && op->p1 == (i == 0 ? 0 : (i == 1 ? k * 100 : (i == 2 ? 1 : k * 100)))
                && op->p2 == (i == 0 ? k * 100 : (i == 1 ? 0 : (i == 2 ? k * 100 + 1 : 2)))
                && op->p3 == (i == 0 ? 5 : (i == 1 ? 0 : (i == 2 ? 0 : 1)))
                && op->p5 == 0 && op->p4 == 0;
            if (!ok) {
                printf("v[%d].aOp[%d] corrupt: opcode=%u p4type=%d p1=%u p2=%u p3=%u p5=%u p4=%lld\n",
                       k, i, op->opcode, op->p4type, op->p1, op->p2, op->p3, op->p5, op->p4);
                bad++;
            }
        }
    }
    if (bad) { printf("FAILED (%d)\n", bad); return 1; }
    printf("OK\n");
    return 0;
}
