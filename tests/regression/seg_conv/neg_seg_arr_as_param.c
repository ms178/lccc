long __seg_gs arr[4];
void g(long *);
void f(void) { g(arr); }
