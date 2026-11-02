/* compare-replay. When a Cmp's single use is a
 * Select that is NOT adjacent to the Cmp (an ALU instruction such as
 * `sub` sits between, clobbering the flags), the Cmp must skip the boolean
 * materialization (setcc/movzbl) and the Select must re-emit the compare +
 * cmovcc directly. A wrong implementation either doubles the compare or
 * reads a stale/materialized boolean.
 *
 * Semantics are checked against a scalar reference across boundary values
 * (exactly the fill_window `m >= WSIZE ? m - WSIZE : 0` shape plus a
 * non-trivial intermediate computation between compare and select). */
typedef unsigned short Pos;
#define WSIZE 32768
#define NIL 0
static Pos arr[WSIZE];

static unsigned slide2(void) {
    unsigned n, m, acc = 0;
    for (n = 0; n < WSIZE; n++) {
        m = arr[n];
        unsigned t = m - WSIZE;          /* between Cmp and Select */
        arr[n] = (Pos)(m >= WSIZE ? t : NIL);
        acc += arr[n];
    }
    return acc;
}

int main(void) {
    for (unsigned i = 0; i < WSIZE; i++) arr[i] = (Pos)((i * 5 + 7) % (WSIZE + 12345));
    unsigned a = slide2();
    unsigned ref = 0;
    for (unsigned i = 0; i < WSIZE; i++) {
        unsigned m = (unsigned)((i * 5 + 7) % (WSIZE + 12345));
        ref += (m >= WSIZE) ? (m - WSIZE) : 0;
    }
    return a == ref ? 0 : 1;
}
