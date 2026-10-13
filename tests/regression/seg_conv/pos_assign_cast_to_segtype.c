/* The cast names the segment, so the right-hand side already has the right space. */
typedef int __seg_gs *gp_t;
int *q;
gp_t gp;
void f(void) { gp = (int __seg_gs *)q; }
