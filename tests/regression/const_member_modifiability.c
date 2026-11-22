/* const_member_modifiability.c -- C11 6.3.2.1p3 modifiable-lvalue boundaries
 * that must stay ACCEPTED.  An lvalue of struct/union type is non-modifiable
 * when any member is const-qualified recursively -- but a plain member whose
 * own type carries no const IS writable even inside such an aggregate, and
 * aggregates without any const member stay fully assignable.  Initialization
 * (unlike assignment) of const-containing aggregates is always legal.  The
 * rejected half of the boundary (assignment of read-only member/variable/
 * location) is pinned by tests/regression/check_sema_lvalue_gates.sh against
 * GCC 16.2's exact diagnostics.
 */
#include <stdio.h>

struct s6 { const int v; };
struct s5 {
    union {
        struct s6 n;
        int z;
    } u;
};
struct clean {
    int a, b;
};
struct has_const {
    int z;
    const int c;
};
struct wrap {
    struct clean m;
};

int main(void)
{
    int fails = 0;
    struct s5 wu = { 0 };
    struct s6 w = { 1 };            /* initialization with const member: legal */
    struct clean o = { 3, 4 }, o2 = { 0, 0 };
    struct has_const hc = { 8, 9 };
    struct wrap wp;
    struct s5 *p = &wu;
    struct clean arr[2];
    struct s5 copy = wu;           /* initialization (not assignment): legal */
    (void)copy;

    /* Plain int member of a const-containing union: writable (measured
     * GCC 16.2 -- the member's own type is unqualified `int`). */
    wu.u.z = 5;
    if (wu.u.z != 5) {
        printf("FAIL plain member write\n");
        fails++;
    }
    p->u.z += 1;
    if (wu.u.z != 6) {
        printf("FAIL plain member compound write\n");
        fails++;
    }

    /* Plain member next to a const member. */
    hc.z = 7;
    if (hc.z != 7 || hc.c != 9) {
        printf("FAIL sibling member write\n");
        fails++;
    }

    /* Clean aggregates: full assignment is legal. */
    o.a = 9;
    o = o2;
    if (o.a != 0 || o.b != 0) {
        printf("FAIL clean struct assign\n");
        fails++;
    }
    wp.m = o;
    arr[0] = o;
    arr[1].b = 6;
    if (wp.m.a != 0 || arr[1].b != 6) {
        printf("FAIL nested/array member assign\n");
        fails++;
    }

    /* Initialization of a const-containing aggregate is NOT assignment:
     * `struct s5 copy = wu;` above is legal C11 where `copy = wu;` would
     * be a constraint violation (pinned as a rejection in the gates). */
    if (fails == 0) {
        printf("ALL-OK\n");
    }
    return fails == 0 ? 0 : 1;
}
