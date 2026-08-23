/* gcc.c-torture/execute/20070212-2.c + 20080604-1.c reduced.
 * Aggregate/field DSE must treat pointer phis that merge multiple roots, or a
 * tracked alloca with an untracked global, as escapes. Stores through such phis
 * may be read later and are not dead.
 */
extern void abort(void);
struct S { const char *p; } g;
int choose_local(int k, int a, int b) {
    int *p = k ? &a : &b;
    a = 0;
    return *p;
}
void write_global_via_phi(int k) {
    struct S local;
    struct S *p = k ? &local : (&g + 1) - 1;
    p->p = "ok";
}
int main(void) {
    if (choose_local(1, 1, 2) != 0) abort();
    g.p = 0;
    write_global_via_phi(0);
    if (!g.p) abort();
    return 0;
}
