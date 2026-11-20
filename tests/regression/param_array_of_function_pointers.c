/* A parameter declared as an array of pointers to function (or to array) is
 * adjusted to a pointer to its element, C11 6.7.6.3p7:
 *
 *     int (*h[])(struct S *)   is   int (**h)(struct S *)
 *     int (*h[2])[4]           is   int (**h)[4]
 *
 * LCCC dropped the array and typed the parameter as ONE pointer to function,
 * so every table passed to it was a constraint violation, and `h[i]` in the
 * body was arithmetic on a function.  Linux hands a table of exit handlers to
 * `nested_vmx_hardware_setup(int (*exit_handlers[])(struct kvm_vcpu *))`
 * (arch/x86/kvm/vmx/nested.c); KVM did not compile.
 */
#include <stdio.h>

struct S {
    int v;
};

static int add1(struct S *s) { return s->v + 1; }
static int add2(struct S *s) { return s->v + 2; }
static int twice(struct S *s) { return s->v * 2; }
static int inc(int x) { return x + 1; }
static int dec(int x) { return x - 1; }
static int neg(int x) { return -x; }
static int *pick(int *a, int i) { return a + i; }

/* Every spelling of the same parameter type. */
__attribute__((noinline)) int unsized(int (*h[])(struct S *), struct S *s, int n)
{
    int r = 0;
    for (int i = 0; i < n; i++)
        r = r * 10 + h[i](s);
    return r;
}
__attribute__((noinline)) int sized(int (*h[3])(struct S *), struct S *s)
{
    return h[0](s) + (*h[1])(s) * 100 + (**(h + 2))(s) * 10000;
}
__attribute__((noinline)) int with_static(int (*h[static 2])(int), int x)
{
    return h[0](x) * 1000 + h[1](x);
}
__attribute__((noinline)) int qualified(int (*h[const 3])(int), int x)
{
    return h[2](h[1](h[0](x)));
}
__attribute__((noinline)) int stepping(int (*h[])(int), int n, int x)
{
    /* The parameter is a pointer object: it can be moved. */
    while (n--)
        x = (*h++)(x);
    return x;
}
__attribute__((noinline)) int returns_pointer(int *(*h[])(int *, int), int *a)
{
    return *h[0](a, 1) + *h[1](a, 2) * 10;
}
__attribute__((noinline)) int rows(int (*h[])[4], int i, int j)
{
    return (*h[i])[j] + (int)sizeof(*h[0]) * 100;
}
__attribute__((noinline)) int two_rows(int (*h[2])[4])
{
    return (*h[0])[3] * 10 + (*h[1])[0];
}
/* The explicit spellings these adjust to, so both must agree. */
__attribute__((noinline)) int explicit_functions(int (**h)(struct S *), struct S *s, int n)
{
    return unsized(h, s, n);
}
__attribute__((noinline)) int explicit_rows(int (**h)[4], int i, int j)
{
    return rows(h, i, j);
}

/* Compatible redeclarations: each pair names one type. */
int unsized(int (**)(struct S *), struct S *, int);
int sized(int (*[])(struct S *), struct S *);
int rows(int (**)[4], int, int);

int main(void)
{
    struct S s = { 4 };
    int (*table[3])(struct S *) = { add1, add2, twice };
    int (*ints[3])(int) = { inc, neg, dec };
    int *(*pickers[2])(int *, int) = { pick, pick };
    int values[4] = { 7, 8, 9, 6 };
    int row_a[4] = { 1, 2, 3, 4 }, row_b[4] = { 5, 6, 7, 8 };
    int (*row_table[2])[4] = { &row_a, &row_b };

    printf("unsized %d\n", unsized(table, &s, 3));
    printf("sized %d\n", sized(table, &s));
    printf("static %d\n", with_static(ints, 5));
    printf("qualified %d\n", qualified(ints, 5));
    printf("stepping %d\n", stepping(ints, 3, 5));
    printf("pointer %d\n", returns_pointer(pickers, values));
    printf("rows %d %d\n", rows(row_table, 0, 2), rows(row_table, 1, 3));
    printf("two_rows %d\n", two_rows(row_table));
    printf("explicit %d %d\n", explicit_functions(table, &s, 2), explicit_rows(row_table, 1, 1));
    /* The adjusted parameter is one pointer wide whatever the bound says. */
    printf("size %d\n", (int)(sizeof(int (**)(int)) == sizeof(void *)));
    printf("generic %d\n", _Generic(&table[0], int (**)(struct S *): 1, default: 0));
    return 0;
}
