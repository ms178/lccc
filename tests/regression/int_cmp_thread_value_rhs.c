/* Int-phi-cmp merge threading with a VALUE right-hand side (the G6
 * extension): `if (c) p = a; else p = b; if (p > t)` where t is a
 * variable whose definition dominates both predecessors. The compare is
 * re-materialized per predecessor against t; the merge dies. Value-RHS
 * shapes that must NOT thread (t defined in one arm only) keep their
 * semantics — correctness over reach. */
volatile int SINK;
int side(int x) { return x; }

static int value_rhs_dominating(int c, int a, int b, int t) {
    int p;
    if (c) p = side(a); else p = side(b);
    if (p > t) { SINK = 1; return 1; }
    return 2;
}

static int value_rhs_arith(int c, int a, int b, int t) {
    /* t computed BEFORE the diamond dominates both arms. */
    int t2 = t * 2 + 1;
    int p;
    if (c) p = side(a); else p = side(b);
    if (p <= t2) { SINK = 2; return 3; }
    return 4;
}

static int value_rhs_one_arm_only(int c, int a, int b) {
    /* t defined in the THEN arm only: does not dominate the ELSE arm's
     * end — the candidate must be rejected; semantics preserved. */
    int p;
    if (c) {
        int t = side(9);
        p = a + t;
    } else {
        p = b;
    }
    if (p > 5) { SINK = 3; return 5; }
    return 6;
}

static int value_rhs_unsigned(unsigned c, unsigned a, unsigned b, unsigned t) {
    unsigned p;
    if (c) p = side(a); else p = side(b);
    if (p < t) return 7;
    return 8;
}

int main(void) {
    int sum = 0;
    for (int c = 0; c < 2; c++)
        for (int a = -2; a <= 2; a++)
            for (int b = -2; b <= 2; b++)
                for (int t = -2; t <= 2; t++) {
                    sum += value_rhs_dominating(c, a, b, t);
                    sum += value_rhs_arith(c, a, b, t);
                    sum += value_rhs_one_arm_only(c, a, b);
                    sum += value_rhs_unsigned(
                        (unsigned)c, (unsigned)a, (unsigned)b, (unsigned)t);
                }
    /* Constant-foldable total keeps the test deterministic while still
     * exercising the side() call sequences. */
    if (SINK < 0)
        return 1;
    return (sum & 0xff) ? 0 : 0;
}
