struct S { long a[4]; };
struct S __seg_gs gobj;
void g(long *);
void f(void) { g(gobj.a); }
