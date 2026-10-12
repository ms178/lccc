/* (char *)0 is not the void * exemption: GCC 14.2 rejects it as an argument. */
typedef int __seg_gs *gp_t;
void take(gp_t p);
void f(void) { take((char *)0); }
