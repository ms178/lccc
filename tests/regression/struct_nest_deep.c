/* struct_nest_deep.c -- deep nested record definitions and member shapes.
 *
 * The LIM/RBR X-macro ladder (the gcc.c-torture/compile/limits-structnest
 * shape) builds a 10,000-level named struct chain whose every level declares
 * a member `x`; the innermost field is an int and the outermost `} x;` chain
 * declares the STRUCT variable `x` itself.  The record-level trampoline makes
 * that parse in constant stack and the sema depth guard (16384) sits well
 * above it, so this must compile and run on both x86-64 and i686 with output
 * identical to GCC 16.2.  The named s6..s0 chain pins union, const-member,
 * multi-declarator, array-member and pointer-member resolution through
 * `field_ctype`.
 *
 * `x` is the deep-chain STRUCT variable: assertions on it are size/offset
 * checks only (`sizeof x >= 4`), never int assignment.  The chain stays on
 * the token-pasted NAMED pattern deliberately: 10k-deep ANONYMOUS nesting
 * is gcc-quadratic (2m09s measured vs 1.9s for this shape) and would blow
 * the runner's 90s oracle budget without testing anything extra.
 */
#include <stdio.h>

#define LIM1(x) x##0 {x##1 {x##2 {x##3 {x##4 {x##5 {x##6 {x##7 {x##8 {x##9 {
#define LIM2(x) LIM1(x##0) LIM1(x##1) LIM1(x##2) LIM1(x##3) LIM1(x##4) \
		LIM1(x##5) LIM1(x##6) LIM1(x##7) LIM1(x##8) LIM1(x##9)
#define LIM3(x) LIM2(x##0) LIM2(x##1) LIM2(x##2) LIM2(x##3) LIM2(x##4) \
		LIM2(x##5) LIM2(x##6) LIM2(x##7) LIM2(x##8) LIM2(x##9)
#define LIM4(x) LIM3(x##0) LIM3(x##1) LIM3(x##2) LIM3(x##3) LIM3(x##4) \
		LIM3(x##5) LIM3(x##6) LIM3(x##7) LIM3(x##8) LIM3(x##9)

#define RBR1 } x; } x; } x; } x; } x; } x; } x; } x; } x; } x;
#define RBR2 RBR1 RBR1 RBR1 RBR1 RBR1 RBR1 RBR1 RBR1 RBR1 RBR1
#define RBR3 RBR2 RBR2 RBR2 RBR2 RBR2 RBR2 RBR2 RBR2 RBR2 RBR2
#define RBR4 RBR3 RBR3 RBR3 RBR3 RBR3 RBR3 RBR3 RBR3 RBR3 RBR3

/* 10,000 nested named structs; innermost field `int x`, outermost `} x;`
 * declares the struct variable `x`. */
LIM4(struct s)
  int x;
RBR4

/* Named chain: const leaf inside a union, const member, multi-declarator,
 * array member, pointer member. */
struct s6 { const int v; };
struct s5 { union { struct s6 n; int i; } u; };
struct s4 { const struct s5 s; };
struct s3 { struct s4 a, b; };
struct s2 { struct s3 arr[2]; };
struct s1 { struct s2 *p; };
struct s0 { struct s1 m; int tag; };

/* Anonymous struct object, pointer+array derived types, multi-declarator. */
static struct { int a, b; } ga = { 3, 4 };
static int (*gp)[3];
static int gm1, gm2[2], *gm3;

int main(void)
{
    int fails = 0;

    /* Deep chain sanity: compile/link-level proof plus a size fact. */
    if (!(sizeof x >= 4)) {
        printf("FAIL sizeof x\n");
        fails++;
    }
    printf("deep sizeof=%zu\n", sizeof x);

    /* Anonymous struct object. */
    if (ga.a != 3 || ga.b != 4) {
        printf("FAIL ga\n");
        fails++;
    }

    /* Pointer-to-array member type. */
    {
        int carr[3] = { 1, 2, 3 };
        gp = &carr;
        if ((*gp)[2] != 3) {
            printf("FAIL gp\n");
            fails++;
        }
    }

    /* Multi-declarator globals. */
    gm1 = 11;
    gm2[1] = 22;
    gm3 = &gm1;
    if (gm1 != 11 || gm2[1] != 22 || *gm3 != 11) {
        printf("FAIL gm\n");
        fails++;
    }

    /* Union member write/read of the same member (no punning). */
    {
        struct s5 u;
        u.u.i = 5;
        if (u.u.i != 5) {
            printf("FAIL union rw\n");
            fails++;
        }
    }

    /* Const-containing chain: build by initializer, read through every
     * level (const members make these types non-assignable on purpose). */
    {
        struct s4 c4 = { .s = { .u = { .n = { 77 } } } };
        struct s3 pair = {
            .a = { .s = { .u = { .n = { 77 } } } },
            .b = { .s = { .u = { .n = { 88 } } } },
        };
        struct s2 row = {
            .arr = {
                [0] = { .a = { .s = { .u = { .n = { 11 } } } },
                        .b = { .s = { .u = { .n = { 12 } } } } },
                [1] = { .a = { .s = { .u = { .n = { 21 } } } },
                        .b = { .s = { .u = { .n = { 22 } } } } },
            },
        };
        struct s0 top;

        if (c4.s.u.n.v != 77) {
            printf("FAIL s4 const chain\n");
            fails++;
        }
        if (pair.a.s.u.n.v != 77 || pair.b.s.u.n.v != 88) {
            printf("FAIL s3 multi-declarator\n");
            fails++;
        }
        if (row.arr[0].a.s.u.n.v != 11 || row.arr[1].b.s.u.n.v != 22) {
            printf("FAIL s2 array member\n");
            fails++;
        }

        top.m.p = &row;
        top.tag = 9;
        if (top.m.p->arr[1].b.s.u.n.v != 22 || top.tag != 9) {
            printf("FAIL s0 pointer member\n");
            fails++;
        }
    }

    if (fails == 0)
        printf("ALL-OK\n");
    return fails != 0;
}
