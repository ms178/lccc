typedef __seg_gs int *gp_t;
int *q;
struct U { struct { gp_t p; }; int z; };
void n7(void) { struct U lu = { .z = 1, 2, q }; (void)lu; }
void n8(void) { struct U lu = { .z = 1, q, 2 }; (void)lu; }
void n9(void) { struct U lu = { .p = q }; (void)lu; }
