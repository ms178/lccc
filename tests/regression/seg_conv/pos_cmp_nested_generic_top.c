/* Only the top-level pointee space is compared: int __seg_gs ** vs int ** is accepted. */
int __seg_gs **a;
int **b;
int f(void) { return a == b; }
