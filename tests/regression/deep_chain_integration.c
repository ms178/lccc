/* deep_chain_integration.c -- cross-feature interference coverage.
 *
 * flat label runs (switch dispatch), paren-chain peel (expressions) and the
 * nested-record trampoline (member resolution) each have their own fixture;
 * this pins their INTERACTION: label-run runs whose case bodies contain
 * peeled paren chains on nested-member operands, in one TU, under -O0 and
 * -O2, on x86-64 and i686, byte-identical to GCC 16.2.  Interference bugs
 * (shared depth state, peel-vs-label dispatch order, field_ctype caching
 * across switch arms) would show up only here.
 */
#include <stdio.h>

#define LBR1 (
#define RBR1 )
#define LBR2 LBR1 LBR1 LBR1 LBR1 LBR1 LBR1 LBR1 LBR1 LBR1 LBR1
#define RBR2 RBR1 RBR1 RBR1 RBR1 RBR1 RBR1 RBR1 RBR1 RBR1 RBR1

/* Named nested struct chain, shallow enough for GCC's oracle budget but
 * exercising the same member-resolution path as struct_nest_deep. */
struct in3 { int v; };
struct in2 { struct in3 n; };
struct in1 { struct in2 n; };
struct in0 { struct in1 n; };
struct out {
    struct in0 root;
    struct in3 flat;
};

/* X-macro case runs on distinct decade bases. */
#define R10(b) \
    case (b): case (b)+1: case (b)+2: case (b)+3: case (b)+4: \
    case (b)+5: case (b)+6: case (b)+7: case (b)+8: case (b)+9:

static int dispatch(int key, struct out *o)
{
    switch (key) {
    R10(0)
        /* Peeled paren chain on a nested-member operand. */
        return LBR2 o->root.n.n.n.v RBR2;
    R10(10)
        return LBR1 LBR1 o->flat.v RBR1 RBR1 + 1;
    R10(20)
        return LBR2 o->flat.v == 3 RBR2 ? o->root.n.n.n.v * 2 : -99;
    default:
        return -1;
    }
}

int main(void)
{
    struct out o;
    o.root.n.n.n.v = 7;
    o.flat.v = 3;
    int fails = 0;
    int sum = 0;
    int k;
    for (k = 0; k < 30; k++) {
        sum += dispatch(k, &o);
    }
    /* decades 0/10/20 each return a constant, ten times */
    if (sum != 10 * 7 + 10 * 4 + 10 * 14) {
        printf("FAIL sum=%d\n", sum);
        fails++;
    }
    if (dispatch(77, &o) != -1) {
        printf("FAIL default\n");
        fails++;
    }
    if (fails == 0) {
        printf("ALL-OK\n");
    }
    return fails == 0 ? 0 : 1;
}
