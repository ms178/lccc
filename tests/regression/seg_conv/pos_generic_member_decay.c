/* Generic (non-segment) array decay through members, returns and assignments
 * must keep converting: the decayed pointer has no named address space. */
struct S { long a[4]; };
struct S gs_plain;
void g(long *p);
long *ret_member(void) { return gs_plain.a; }
void arg_member(void) { g(gs_plain.a); }
void assign_member(void) { long *q; q = gs_plain.a + 1; g(q); }
