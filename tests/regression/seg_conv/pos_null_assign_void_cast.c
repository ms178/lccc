/* GCC 14.2 accepts a void * null pointer constant assigned or initialised. */
typedef int __seg_gs *gp_t;
gp_t gp;
gp_t gi = (void *)0;
void f(void) { gp = (void *)0; }
