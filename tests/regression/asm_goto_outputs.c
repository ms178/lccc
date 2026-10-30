#include <stddef.h>
#include <stdio.h>

/* A label branch and a GNU flag output read the same hardware flags. Keep the
 * branch condition deliberately different from the output condition: this
 * exercises true/false outputs on both the fall-through and asm-goto edges. */
static volatile int observed;

#define DEFINE_CC_CASE(name, suffix, expected_expr)                         \
    static int check_##name(int x) {                                        \
        _Bool out;                                                          \
        int path;                                                           \
        asm goto volatile("testl %1, %1; jz %l[target]"                   \
                          : "=@cc" #suffix(out)                           \
                          : "r"(x)                                        \
                          : "cc"                                          \
                          : target);                                       \
        if (out) { observed = 1; path = 10; }                              \
        else { observed = 0; path = 11; }                                  \
        goto done;                                                          \
    target:                                                                 \
        if (out) { observed = 1; path = 20; }                              \
        else { observed = 0; path = 21; }                                  \
    done:                                                                   \
        return path;                                                        \
    }

DEFINE_CC_CASE(c, c, 0)
DEFINE_CC_CASE(nc, nc, 1)
DEFINE_CC_CASE(b, b, 0)
DEFINE_CC_CASE(ae, ae, 1)
DEFINE_CC_CASE(be, be, (x == 0))
DEFINE_CC_CASE(a, a, (x != 0))
DEFINE_CC_CASE(e, e, (x == 0))
DEFINE_CC_CASE(z, z, (x == 0))
DEFINE_CC_CASE(ne, ne, (x != 0))
DEFINE_CC_CASE(nz, nz, (x != 0))
DEFINE_CC_CASE(g, g, (x > 0))
DEFINE_CC_CASE(ge, ge, (x >= 0))
DEFINE_CC_CASE(l, l, (x < 0))
DEFINE_CC_CASE(le, le, (x <= 0))
DEFINE_CC_CASE(s, s, (x < 0))
DEFINE_CC_CASE(ns, ns, (x >= 0))
DEFINE_CC_CASE(o, o, 0)
DEFINE_CC_CASE(no, no, 1)
DEFINE_CC_CASE(p, p, (__builtin_parity((unsigned char)x) == 0))
DEFINE_CC_CASE(np, np, (__builtin_parity((unsigned char)x) != 0))
DEFINE_CC_CASE(nbe, nbe, (x != 0))
DEFINE_CC_CASE(nb, nb, 1)
DEFINE_CC_CASE(nae, nae, 0)
DEFINE_CC_CASE(na, na, (x == 0))
DEFINE_CC_CASE(ng, ng, (x <= 0))
DEFINE_CC_CASE(nge, nge, (x < 0))
DEFINE_CC_CASE(nl, nl, (x >= 0))
DEFINE_CC_CASE(nle, nle, (x > 0))

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
    int want_out = !!(expr);                                                \
    int want_path = (x) == 0                                                \
        ? (want_out ? 20 : 21)                                              \
        : (want_out ? 10 : 11);                                             \
    int got_path = check_##name(x);                                         \
    int got_out = observed;                                                 \
    if (got_path != want_path || got_out != want_out) {                     \
        printf("FAIL %s x=%d path=%d/%d out=%d/%d\n", #name, (x),         \
               got_path, want_path, got_out, want_out);                     \
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
        return value == 42 ? 42 : -1;
    return value == 42 ? 24 : -2;
zero:
    return value == 42 && is_zero ? 42 : -3;
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
        return value == 7 ? 42 : -1;
    return value == (unsigned)(7 + x) ? 24 : -2;
zero:
    return value == 7 && is_zero ? 42 : -3;
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
    return value == 73 ? 1 : -2;
negative:
    return value == 73 ? 2 : -3;
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
    return value == 61 ? 1 : -2;
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
    if (out)
        return 20;
    return 21;
}

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
        if (check_multiple_outputs(x) != (x == 0 ? 42 : 24)) {
            printf("FAIL multiple outputs x=%d\n", x);
            return 1;
        }
        if (check_tied_output(x) != (x == 0 ? 42 : 24)) {
            printf("FAIL tied output x=%d\n", x);
            return 1;
        }
        if (check_two_target_output(x) != (x == 0 ? 1 : x < 0 ? 2 : 3)) {
            printf("FAIL two-target output x=%d\n", x);
            return 1;
        }
        if (check_numeric_label_output(x) != (x == 0 ? 1 : 3)) {
            printf("FAIL numeric-label output x=%d\n", x);
            return 1;
        }
    }
    if (check_shared_target(0, 1) != 21
        || check_shared_target(0, 0) != 20
        || check_shared_target(1, 0) != 11) {
        puts("FAIL shared target predecessor");
        return 1;
    }
    if (asm_cc_output_branch(0) != 2 || asm_cc_output_branch(1) != 0) {
        puts("FAIL condition-code branch output");
        return 1;
    }
    if (asm_cc_output_volatile(0) != 2 || asm_cc_output_volatile(1) != 0) {
        puts("FAIL volatile condition-code output");
        return 1;
    }
    puts("PASS asm_goto_outputs");
    return 0;
}
