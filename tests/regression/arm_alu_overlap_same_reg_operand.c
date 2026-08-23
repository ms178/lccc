/* AArch64 regalloc may use half-open endpoint handoffs, but two different
 * values used by the same ALU instruction cannot be read from the same physical
 * register. Codegen must reload one operand from its stack home. Reduced from
 * gcc.c-torture/execute/20120919-1.c.
 */
extern void abort(void);
volatile int sink[64];
__attribute__((noinline)) int add_after_pressure(int n, int *p) {
    int s = 0;
    for (int i = -1; i < n; ++i) {
        if (i == 0) {
            int x1=sink[1], x2=sink[2], x3=sink[3], x4=sink[4], x5=sink[5], x6=sink[6];
            int x7=sink[7], x8=sink[8], x9=sink[9], x10=sink[10], x11=sink[11], x12=sink[12];
            s += p[i];
            sink[0] = x1+x2+x3+x4+x5+x6+x7+x8+x9+x10+x11+x12;
        }
    }
    return s;
}
int main(void) {
    int a[1] = {1234567890};
    if (add_after_pressure(1, a) != 1234567890) abort();
    return 0;
}
