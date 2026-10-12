/* A row of a two-dimensional named-space array decays into that space. */
long __seg_gs grid[3][4];
void f(void) { long *p = grid[1]; (void)p; }
