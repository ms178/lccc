/*
 * Variably modified (VM) types reached through expressions (C11 6.5.3.4p2,
 * 6.7.6.2, 6.8p3; C23 6.7.3.6p4 typeof, 6.7.10 auto / GNU __auto_type).
 *
 * A VLA's size and strides exist only as values computed where its
 * declaration is reached.  Before the fix, lowering lost them whenever a
 * VM type was reached indirectly:
 *   - through a pointer to a VLA (`int (*p)[n]`): p[i][j], p++, p += k,
 *     sizeof *p and sizeof p[i] used the pointer size as the row size;
 *   - through typeof/typedef/__auto_type: typeof(m[1]) sized 4, not 4*k;
 *   - `sizeof m[f()]` and `typeof(m[f()])` never called f;
 *   - __auto_type from any array did not decay (sizeof 48, garbage reads).
 * Expected output is GCC 16.2's (Clang 23.1.3 agrees).
 */
#include <stdio.h>

static int calls;
static int f(void) { return calls++, 1; }

/* Pointer-to-VLA parameters and locals: subscripts, sizeof, arithmetic. */
static int idx(int n, int (*q)[n], int i, int j) { return q[i][j]; }
static size_t sz(int n, int (*q)[n]) { return sizeof *q + sizeof q[1]; }
static int bound_calls;
static int three(void) { return bound_calls++, 3; }
static int vm_param(int (*q)[three()]) { return (int)sizeof *q; }

static void pointer_to_vla(void)
{
    int m[3][5];
    for (int i = 0; i < 15; i++)
        m[i / 5][i % 5] = i;
    int n = 5;
    int (*p)[n] = m;
    printf("%d %zu %d %zu %zu\n", idx(5, m, 2, 3), sz(5, m), p[1][4], sizeof *p, sizeof p[2]);
    p++;
    printf("%d %d\n", (*p)[0], *(*(p + 1) + 2));

    int b[3];
    int r = vm_param(&b);
    int c1 = bound_calls;
    int (*pb)[three()] = &b; /* the bound is evaluated once, here */
    printf("%d %d %d %zu\n", r, c1, bound_calls, sizeof *pb);

    int k = 4, mm[3][4] = {{0, 1, 2, 3}, {4, 5, 6, 7}, {8, 9, 10, 11}};
    int (*q)[k] = mm;
    q++; ++q; q -= 1; q += 1;
    int (*e)[k] = mm + 3;
    printf("%d %td %td %d\n", q[0][1], e - q, q - mm, (*(q - 1))[3]);
}

/* typeof, typedef and __auto_type of VM types; sizeof operand evaluation. */
static void typeof_vm(void)
{
    int n = 3, k = 4;
    int m[n][k];
    int v[n];
    int (*p)[k] = m;
    for (int i = 0; i < n; i++)
        for (int j = 0; j < k; j++)
            m[i][j] = 10 * i + j;
    typeof(v) t1;
    typeof(m) t2;
    typeof(m[1]) t3;
    typeof(*p) t4;
    typeof(m[f()]) t5; /* VM operand: evaluated */
    typeof(int[n]) t6;
    __typeof__(p) t7 = m;
    typedef typeof(m[0]) Row;
    Row r;
    printf("%zu %zu %zu %zu %zu %zu %zu %td calls=%d\n", sizeof t1, sizeof t2, sizeof t3,
           sizeof t4, sizeof t5, sizeof t6, sizeof r, (char *)&t7[1] - (char *)t7, calls);

    for (int i = 0; i < n; i++)
        for (int j = 0; j < k; j++)
            t2[i][j] = m[i][j] + 100;
    typeof(m[0]) *rp = m;
    typeof(m[0]) a2[2];
    a2[1][3] = 7;
    typeof(&m[0]) pp = &m[1];
    __auto_type ap = m;
    printf("%d %d %zu %d %zu %d %d\n", t2[2][3], rp[2][1], sizeof a2, a2[1][3], sizeof *rp,
           pp[1][0], ap[2][2]);

    size_t s1 = sizeof m[f()];        /* VM operand: evaluated */
    size_t s2 = sizeof(int[f()]);     /* VLA type-name: bound evaluated */
    size_t s3 = _Alignof(int[f()]);   /* not evaluated */
    size_t s4 = sizeof(m[0][f()]);    /* int: not evaluated */
    int x = 0;
    size_t s5 = sizeof(m[x++]);       /* evaluated */
    size_t s6 = sizeof(m[0][x++]);    /* not evaluated */
    printf("%zu %zu %zu %zu calls=%d %zu %zu x=%d\n", s1, s2, s3, s4, calls, s5, s6, x);
}

/* __auto_type takes the initializer's type after decay. */
static void auto_type_decay(void)
{
    int a[3][4] = {{0, 1, 2, 3}, {10, 11, 12, 13}, {20, 21, 22, 23}};
    __auto_type ap = a;
    int v[5] = {1, 2, 3, 4, 5};
    __auto_type vp = v;
    __auto_type fp = f;
    printf("%d %d %d %d %d\n", ap[2][2], sizeof ap == sizeof(int (*)[4]), vp[4],
           sizeof vp == sizeof(int *), fp == f);
}

int main(void)
{
    pointer_to_vla();
    typeof_vm();
    auto_type_decay();
    return 0;
}
