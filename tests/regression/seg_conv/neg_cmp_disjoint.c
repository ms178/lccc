/* GCC 14.2: comparison of pointers to disjoint address spaces. */
int *q;
typedef int __seg_gs *gp_t;
gp_t gp;
int f(void) { return gp == q; }
