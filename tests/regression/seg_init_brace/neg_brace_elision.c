typedef __seg_gs int *gp_t;
int *q;
struct T { int n; struct { int y; gp_t p; } inner; };
struct U { struct { gp_t p; }; int z; };
union V { gp_t p; int i; };
struct S { gp_t p; int n; };
gp_t garr[2] = { [1] = q };
void n1(void) { struct T lt = { 1, 2, q }; (void)lt; }
void n2(void) { struct U lu = { q, 5 }; (void)lu; }
void n3(void) { struct T l3 = { .inner.p = q }; (void)l3; }
void n4(void) { union V lv = { q }; (void)lv; }
void n5(void) { gp_t la[2] = { [1] = q }; (void)la; }
void n6(void) { struct S ls = { q, 1 }; (void)ls; }
void n7(void) { struct U lu = { .z = 1, q }; (void)lu; }
void n8(void) { struct T lt = { 1, { 2, q } }; (void)lt; }
void n9(void) { struct T lt = { 1, 2, (gp_t)q }; (void)lt; }
