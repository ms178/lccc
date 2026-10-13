/* A generic array decays to a generic pointer: disjoint from a __seg_gs pointer. */
typedef int __seg_gs *gp_t;
gp_t gp;
int arr[4];
int f(void) { return gp == arr; }
