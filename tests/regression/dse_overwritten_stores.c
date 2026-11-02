/* OP-13: same-block dead store elimination.
 *
 * `x = 1; x = 2; x = 3;` before an escaping use must keep only the final
 * store; an intervening read (via the escaping pointer) keeps every store
 * it can observe. Struct-field stores to disjoint offsets must not
 * interfere, and a store overwritten by a later store to the same field
 * dies. */
#include <stdio.h>

static int sink_val;

__attribute__((noinline)) void sink(int *p) { sink_val = *p; }

/* Only `x = 3` is observable: the earlier stores are overwritten in the
 * same block before &x escapes. */
__attribute__((noinline)) int overwritten(void) {
    int x;
    x = 1;
    x = 2;
    x = 3;
    sink(&x);
    return x;
}

/* sink() reads x BETWEEN the stores: both must survive. */
__attribute__((noinline)) int observed(void) {
    int x;
    x = 10;
    sink(&x);
    x = 20;
    sink(&x);
    return x;
}

struct S { int a, b; };

/* s->a = 1 is overwritten by s->a = 2 before the call; s->b stays. */
__attribute__((noinline)) struct S fields(struct S *s) {
    s->a = 1;
    s->a = 2;
    s->b = 7;
    return *s;
}

int main(void) {
    if (overwritten() != 3 || sink_val != 3) return 1;
    if (observed() != 20) return 2;
    /* sink_val sequence for observed(): 10 then 20 */
    struct S s = {0, 0};
    struct S r = fields(&s);
    if (r.a != 2 || r.b != 7) return 3;
    return 0;
}
