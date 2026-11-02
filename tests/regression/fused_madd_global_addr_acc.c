/* Regression (torture execute/strlen-4.c @O2): fused integer madd with a
 * rematerialisable GlobalAddr accumulator miscompiled to a self-add.
 *
 * The madd emitter holds the product live in %rax across acc staging, but
 * operand_to_callee_reg relayed the homeless GlobalAddr acc through %rax
 * (`leaq sym,%rax; movq %rax,%dest`), destroying the product — the trailing
 * `addq %rax,%dest` then doubled the base (`strlen (2*&a+2)` SIGSEGV).
 *
 * Needs three live checks: the register pressure leaves the mul result
 * homeless (fusion fires) while the add dest stays register-homed.
 */
extern int printf(const char *, ...);
extern __SIZE_TYPE__ strlen(const char *);

typedef char A28[28];
typedef A28 A3_28[3];
typedef A3_28 A2_3_28[2];

static const A2_3_28 a = {
    {"1\00012", "123\0001234", "12345\000123456"},
    {"1234567\00012345678", "123456789\0001234567890",
     "12345678901\000123456789012"},
};

volatile int v0 = 0;
volatile int v1 = 1;
volatile int v2 = 2;

#define A(expr, N)                                                             \
    ((strlen(expr) == N) ? (void)0                                              \
                         : (printf("line %i: strlen (%s = \"%s\") != %i\n",     \
                                   __LINE__, #expr, expr, N),                   \
                            __builtin_abort()))

void test_array_ptr(void) {
    int i0 = 0;
    int i1 = i0 + 1;
    int i2 = i1 + 1;
    int i3 = i2 + 1;

    A(*(&a[i0][i0] + v0) + i1, 0);
    A(*(&a[i0][i0] + v1) + i2, 1);
    A(*(&a[i0][i0] + v2) + i3, 2);
}

int main(void) {
    test_array_ptr();
    return 0;
}
