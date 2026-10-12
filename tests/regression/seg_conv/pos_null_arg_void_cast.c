/* GCC 14.2 accepts a void * null pointer constant for a __seg_gs parameter. */
typedef int __seg_gs *gp_t;
void take(gp_t p);
void f(void) { take((void *)0); take(((void *)0)); }
