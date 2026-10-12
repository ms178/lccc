struct S { long a[4]; };
struct S __seg_gs gobj;
long __seg_gs *p = gobj.a + 1;
