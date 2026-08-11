/* Optimization test (v4): Select emission with a register-resident true
 * value must emit `cmovcc %src, %dest` directly — no dead
 * `movq %src, %rcx` staging copy per select. Semantics checked across a
 * range incl. the aliasing case (true value in the same register as dest
 * would be, forcing the rcx path). */
static int pick(int x) {
    /* true value (x-100) computed before the select, cond uses x */
    int d = x - 100;
    return d > 0 ? d : 0;      /* max(x-100, 0) */
}

int main(void) {
    long acc = 0;
    for (int x = -50; x <= 150; x++) {
        int a = pick(x);
        int ref = x > 100 ? x - 100 : 0;
        if (a != ref) return 1;
        acc += a;
    }
    if (acc != 1275) return 2;   /* sum 1..50 */
    return 0;
}
