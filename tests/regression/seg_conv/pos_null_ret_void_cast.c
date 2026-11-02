/* GCC 14.2 accepts a void * null pointer constant returned as __seg_gs. */
typedef int __seg_gs *gp_t;
gp_t f(void) { return (void *)0; }
