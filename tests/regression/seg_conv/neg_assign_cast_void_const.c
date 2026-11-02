/* (void *)1 is not a null pointer constant: GCC 14.2 rejects it on assignment. */
typedef int __seg_gs *gp_t;
gp_t gp;
void f(void) { gp = (void *)1; }
