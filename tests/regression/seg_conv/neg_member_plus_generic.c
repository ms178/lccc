struct S { long a[4]; };
struct S __seg_gs gobj;
long *p = gobj.a + 1;
