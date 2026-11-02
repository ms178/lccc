/* A void * null pointer constant compares with any named pointer. */
typedef int __seg_gs *gp_t;
gp_t gp;
int f(void) { return gp == (void *)0 || gp != ((void *)0); }
