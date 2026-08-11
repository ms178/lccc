/* Regression/optimization test (v4): IVSR strength reduction must recognize
 * `%x = add %iv, %iv` (the canonical `i * 2` folding produced by the
 * frontend/GVN pipeline) as a stride-2 linear expression. Previously only
 * Mul/Shl were matched, so fill_window-style Pos[] loops (gzip deflate)
 * were not strength-reduced: the index*2 address was re-derived every
 * iteration (~15 instructions/element instead of 9).
 *
 * The test verifies semantics with 32768-element short arrays; the
 * optimization itself is verified by asserting the generated code uses a
 * running pointer (checked by the test driver when CCC_ASM_CHECK is set;
 * semantics are always checked here). */
typedef unsigned short Pos;
#define WSIZE 32768
#define NIL 0
static Pos head[WSIZE], prev[WSIZE];

static unsigned slide(void) {
    unsigned n, m, acc = 0;
    for (n = 0; n < WSIZE; n++) {
        m = head[n];
        head[n] = (Pos)(m >= WSIZE ? m - WSIZE : NIL);
    }
    for (n = 0; n < WSIZE; n++) {
        m = prev[n];
        prev[n] = (Pos)(m >= WSIZE ? m - WSIZE : NIL);
        acc += prev[n];
    }
    return acc;
}

int main(void) {
    /* fill head with values straddling the WSIZE boundary */
    for (unsigned i = 0; i < WSIZE; i++) head[i] = (Pos)((i * 3 + 1000) % (WSIZE + 5000));
    for (unsigned i = 0; i < WSIZE; i++) prev[i] = (Pos)((i * 7 + 42) % (WSIZE + 9000));
    unsigned a = slide();
    /* reference: scalar computation of the transformed prev[] sum */
    unsigned ref = 0;
    for (unsigned i = 0; i < WSIZE; i++) {
        unsigned m = (unsigned)((i * 7 + 42) % (WSIZE + 9000));
        ref += (m >= WSIZE) ? (m - WSIZE) : 0;
    }
    if (a != ref) return 1;
    return 0;
}
