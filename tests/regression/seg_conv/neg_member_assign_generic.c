struct S { long a[4]; };
struct S __seg_gs gobj;
long *p;
void f(void) { p = gobj.a; }
