long __seg_gs arr[4];
void take(long *p) { (void)p; }
void f(void) { take(arr); }
