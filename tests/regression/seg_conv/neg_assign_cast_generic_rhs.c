/* A cast to a generic pointer does not make a generic value legal in a __seg_gs slot. */
typedef int __seg_gs *gp_t;
int *q;
gp_t gp;
void f(void) { gp = (int *)q; }
