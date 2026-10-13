/* The cast's base qualifier names the array the pointer addresses. */
long __seg_gs (*pa)[4];
void f(long *x) { pa = (long __seg_gs (*)[4])x; }
