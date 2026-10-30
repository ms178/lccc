#include <stddef.h>
#include <stdio.h>

/* A label branch and a GNU flag output read the same hardware flags. Keep the
 * branch condition deliberately different from the output condition: this
 * exercises true/false outputs on both the fall-through and asm-goto edges. */
static volatile int observed;

/* GCC defines a `=@cc<suffix>` output immediately after the asm, i.e. on the
 * fall-through edge; on the edge to the label it leaves the object
 * unspecified. This test therefore asserts the flag value only where GCC
 * defines it, and asserts the *label* edge on the path it takes (taken iff
 * x == 0). lccc defines the flag on the label edge as well -- a documented
 * superset -- and asm_goto_outputs_ext.c pins that. */
#define DEFINE_CC_CASE(name, suffix)                                        \
    static int check_##name(int x) {                                        \
        _Bool out;                                                          \
        asm goto volatile("testl %1, %1; jz %l[target]"                     \
                          : "=@cc" #suffix(out)                             \
                          : "r"(x)                                          \
                          : "cc"                                            \
                          : target);                                        \
        if (out) {                                                          \
            observed = 1;                                                   \
            return 10;                                                      \
        }                                                                   \
        observed = 0;                                                       \
        return 11;                                                          \
    target:                                                                 \
        return 20;                                                          \
    }

DEFINE_CC_CASE(c, c)
DEFINE_CC_CASE(nc, nc)
DEFINE_CC_CASE(b, b)
DEFINE_CC_CASE(ae, ae)
DEFINE_CC_CASE(be, be)
DEFINE_CC_CASE(a, a)
DEFINE_CC_CASE(e, e)
DEFINE_CC_CASE(z, z)
DEFINE_CC_CASE(ne, ne)
DEFINE_CC_CASE(nz, nz)
DEFINE_CC_CASE(g, g)
DEFINE_CC_CASE(ge, ge)
DEFINE_CC_CASE(l, l)
DEFINE_CC_CASE(le, le)
DEFINE_CC_CASE(s, s)
DEFINE_CC_CASE(ns, ns)
DEFINE_CC_CASE(o, o)
DEFINE_CC_CASE(no, no)
DEFINE_CC_CASE(p, p)
DEFINE_CC_CASE(np, np)
DEFINE_CC_CASE(nbe, nbe)
DEFINE_CC_CASE(nb, nb)
DEFINE_CC_CASE(nae, nae)
DEFINE_CC_CASE(na, na)
DEFINE_CC_CASE(ng, ng)
DEFINE_CC_CASE(nge, nge)
DEFINE_CC_CASE(nl, nl)
DEFINE_CC_CASE(nle, nle)

/* Keep one compact, noinline shape for a stable codegen check: the condition
 * output is consumed as a C branch both after fall-through and at the asm
 * label, and the target block has no other predecessor. */
__attribute__((noinline)) static int asm_cc_output_branch(unsigned x)
{
    _Bool err;
    asm goto("testl %1, %1; jz %l[label]"
             : "=@ccz"(err)
             : "r"(x)
             : "cc"
             : label);
    if (err)
        return 1;
    return 0;
label:
    if (err)
        return 2;
    return 3;
}

/* Volatile accesses are deliberately not eligible for direct-flag elision. */
__attribute__((noinline)) static int asm_cc_output_volatile(unsigned x)
{
    volatile _Bool err;
    asm goto("testl %1, %1; jz %l[label]"
             : "=@ccz"(err)
             : "r"(x)
             : "cc"
             : label);
    if (err)
        return 1;
    return 0;
label:
    if (err)
        return 2;
    return 3;
}

#define CHECK_CASE(name, expr, x) do {                                      \
    if ((x) == 0) {                                                         \
        /* Taken edge: every reference compiler is unspecified here. */     \
        (void)check_##name(x);                                              \
        break;                                                              \
    }                                                                       \
    int want_out = !!(expr);                                                \
    int want_path = want_out ? 10 : 11;                                     \
    int got_path = check_##name(x);                                         \
    if (got_path != want_path) {                                            \
        printf("FAIL %s x=%d path=%d/%d\n", #name, (x), got_path, want_path); \
        return 1;                                                           \
    }                                                                       \
    if (observed != want_out) {                                             \
        printf("FAIL %s x=%d out=%d/%d\n", #name, (x), observed, want_out); \
        return 1;                                                           \
    }                                                                       \
} while (0)

/* Fall-through-only assertion helper: `x == 0` takes the template branch,
 * whose output values no reference compiler defines, so the call still runs
 * (covering the taken path) but its result is not judged. */
#define CHECK_NONTAKEN(cond, x, call, want) do {                            \
    if (!(cond)) {                                                          \
        (void)(call);                                                       \
        break;                                                              \
    }                                                                       \
    long got_ = (long)(call);                                               \
    if (got_ != (long)(want)) {                                             \
        printf("FAIL %s x=%d got=%ld want=%ld\n", #call, (x), got_,         \
               (long)(want));                                               \
        return 1;                                                           \
    }                                                                       \
} while (0)

static int check_multiple_outputs(int x)
{
    unsigned value = 0;
    _Bool is_zero = 0;
    asm goto volatile("movl $42, %0; testl %2, %2; jz %l[zero]"
                      : "=r"(value), "=@ccz"(is_zero)
                      : "r"(x)
                      : "cc"
                      : zero);
    if (is_zero)
        return -1;
    return value == 42 ? 24 : -2;
zero:
    /* No output object is read on a taken edge: GCC itself disagrees between
     * -O0 and -O2 there, so it defines nothing. asm_goto_outputs_ext.c pins
     * lccc's stronger, deterministic guarantee. */
    return 43;
}

static int check_tied_output(int x)
{
    unsigned value = 7;
    _Bool is_zero = 0;
    asm goto volatile("addl %2, %0; testl %2, %2; jz %l[zero]"
                      : "+r"(value), "=@ccz"(is_zero)
                      : "r"(x)
                      : "cc"
                      : zero);
    if (is_zero)
        return -1;
    return value == (unsigned)(7 + x) ? 24 : -2;
zero:
    return 45;
}

static int check_two_target_output(int x)
{
    unsigned value = 0;
    asm goto volatile("movl $73, %0; testl %1, %1; jz %l[zero]; js %l[negative]"
                      : "+r"(value)
                      : "r"(x)
                      : "cc"
                      : zero, negative);
    return value == 73 ? 3 : -1;
zero:
    return 41;
negative:
    return 42;
}

static int check_numeric_label_output(int x)
{
    unsigned value = 0;
    asm goto volatile("movl $61, %0; testl %1, %1; jz %l3"
                      : "+r"(value)
                      : "r"(x)
                      : "cc"
                      : zero);
    return value == 61 ? 3 : -1;
zero:
    return 44;
}

static int check_shared_target(unsigned x, int bypass)
{
    _Bool out = 0;
    if (bypass)
        goto label;
    asm goto volatile("testl %1, %1; jz %l[label]"
                      : "=@ccz"(out)
                      : "r"(x)
                      : "cc"
                      : label);
    if (out)
        return 10;
    return 11;
label:
    /* Reached by a plain C `goto` as well as by the template branch, so no
     * flag value is observable here on either edge; the point of this case is
     * that a second predecessor disqualifies the fusion. */
    return 21;
}

/* Only the fall-through edge is asserted here. A taken edge is *unspecified*
 * in every reference compiler -- measured: GCC 12 answers f(0) with the
 * fall-through path at -O0 and the label path at -O2, and Clang 22 returns
 * 101 at -O0 versus 244 at -O2 for the same flag read -- so asserting anything
 * there would encode one compiler's accident. lccc defines taken-edge outputs
 * deterministically; asm_goto_outputs_ext.c pins that as the extension it is.
 * The x == 0 inputs below still exercise every taken-edge code path, they just
 * do not judge the value that comes back. */
int main(void)
{
    static const int values[] = { 0, 1, -1, 2, -2, 3, -3,
                                  0x100, 0x101, 0x102, -0x100, -0x101 };
    size_t i;
#define RUN(name, expr, x) CHECK_CASE(name, expr, x)
    for (i = 0; i < sizeof(values) / sizeof(values[0]); ++i) {
        int x = values[i];
        RUN(c, 0, x); RUN(nc, 1, x); RUN(b, 0, x); RUN(ae, 1, x);
        RUN(be, x == 0, x); RUN(a, x != 0, x); RUN(e, x == 0, x);
        RUN(z, x == 0, x); RUN(ne, x != 0, x); RUN(nz, x != 0, x);
        RUN(g, x > 0, x); RUN(ge, x >= 0, x); RUN(l, x < 0, x);
        RUN(le, x <= 0, x); RUN(s, x < 0, x); RUN(ns, x >= 0, x);
        RUN(o, 0, x); RUN(no, 1, x);
        RUN(p, __builtin_parity((unsigned char)x) == 0, x);
        RUN(np, __builtin_parity((unsigned char)x) != 0, x);
        RUN(nbe, x != 0, x); RUN(nb, 1, x); RUN(nae, 0, x);
        RUN(na, x == 0, x); RUN(ng, x <= 0, x); RUN(nge, x < 0, x);
        RUN(nl, x >= 0, x); RUN(nle, x > 0, x);
        /* `check_two_target_output` also branches on `js`, so only x > 0
         * stays on the fall-through path there. */
        CHECK_NONTAKEN(x != 0, x, check_multiple_outputs(x), 24);
        CHECK_NONTAKEN(x != 0, x, check_tied_output(x), 24);
        CHECK_NONTAKEN(x > 0, x, check_two_target_output(x), 3);
        CHECK_NONTAKEN(x != 0, x, check_numeric_label_output(x), 3);
    }
    /* Only the pure C `goto` case is compiler-independent; the asm-edge forms
     * live in asm_goto_outputs_ext.c. */
    if (check_shared_target(0, 1) != 21) {
        puts("FAIL shared target predecessor");
        return 1;
    }
    /* x == 0 takes the template branch, so only the *label* path is defined
     * for the flag object (and only by lccc); x == 1 falls through, where the
     * flag is defined by GCC itself. */
    if (asm_cc_output_branch(1) != 0) {
        puts("FAIL condition-code branch output");
        return 1;
    }
    if (asm_cc_output_volatile(1) != 0) {
        puts("FAIL volatile condition-code output");
        return 1;
    }
    /* The x == 0 (taken-edge) counterparts are judged in
     * asm_goto_outputs_ext.c; here they only have to compile and run. */
    (void)asm_cc_output_branch(0);
    (void)asm_cc_output_volatile(0);
    puts("PASS asm_goto_outputs");
    return 0;
}
