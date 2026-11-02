/* A member array of an element of a named-space array keeps the named space:
 * passing it where a generic pointer is expected is an address-space error. */
struct S { long a[4]; };
struct S __seg_gs sarr[2];
void g(long *p);
void f(void) { g(sarr[0].a); }
