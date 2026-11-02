/* A pointer-to-pointer whose inner layer changes space: GCC 14.2 errors even with -w. */
int __seg_gs **pp;
int **qq;
void f(void) { qq = pp; }
