/* AArch64: spilling an F128 FP parameter calls __trunctfdf2 and clobbers x0.
 * A following GP parameter assigned to x19 must be pre-stored before that call.
 * Reduced from gcc.c-torture/execute/20020413-1.c.
 */
extern void abort(void);
__attribute__((noinline)) void probe(long double val, int *out) {
    long double tmp = 1.0L;
    int i = 0;
    while (tmp < val) {
        tmp *= 2.0L;
        if (++i > 10) abort();
    }
    *out = i;
}
int main(void) {
    int out = -1;
    probe(3.0L, &out);
    if (out != 2) abort();
    return 0;
}
