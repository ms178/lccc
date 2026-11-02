/* gcc.c-torture/execute/20051215-1.c reduced.
 * LICM must not hoist a loop-invariant load out of a conditional block unless
 * the load is must-execute; otherwise a guarded NULL dereference is speculated.
 */
extern void abort(void);
__attribute__((noinline)) int guarded(int n, int *p) {
    int a = 0, b = 0;
    for (int i = 0; i < n; ++i) {
        if (p)
            b = i * *p;
        a += b;
    }
    return a;
}
int main(void) {
    if (guarded(3, 0) != 0) abort();
    return 0;
}
