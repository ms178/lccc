/* After inlining a helper that takes `&x`, stores must hit the alloca
 * itself (not a GEP+0 derived pointer) so mem2reg can promote `x`.
 * Expected: return 0. GCC oracle agrees.
 */
static void set(int *p, int v) { *p = v; }

static int add_into(int *p, int v) {
    *p += v;
    return *p;
}

int main(void) {
    int x = 1;
    set(&x, 40);
    int y = add_into(&x, 2);
    return (x == 42 && y == 42) ? 0 : 1;
}
