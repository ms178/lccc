/* -Wreturn-local-addr: GCC 16.2's front-end walk (c_finish_return), whose
 * verdicts are identical at -O0..-O3 under -fsyntax-only. Each line GCC
 * diagnoses ends in an expect comment; every other line must stay silent.
 * Checked by tests/regression/check_return_local_addr.py. */
struct S { int a[4]; int m; struct S *n; };
struct B { int b : 3; int a[2]; };
int g; int i; long L; int f(void);
extern int w __attribute__((weak));
/* Direct addresses of automatic objects and parameters. */
int *r1(void) { int x = 1; return &x; } /* expect: warning return-local-addr */
int *r2(void) { int a[2] = {0}; return a; } /* expect: warning return-local-addr */
int *r3(void) { struct S s = {0}; return &s.m; } /* expect: warning return-local-addr */
int *r4(void) { struct S s = {0}; return s.a; } /* expect: warning return-local-addr */
int *r5(int p) { return &p; } /* expect: warning return-local-addr */
int *r6(struct S s) { return &s.m; } /* expect: warning return-local-addr */
const int *r7(void) { const int c = 3; return &c; } /* expect: warning return-local-addr */
void *r8(void) { int x = 0; return &x; } /* expect: warning return-local-addr */
int **r9(void) { int *p = &g; return &p; } /* expect: warning return-local-addr */
int *r10(void) { auto int x = 0; return &x; } /* expect: warning return-local-addr */
int *r11(int n) { int a[n]; a[0] = 0; return a; } /* expect: warning return-local-addr */
int *r12(void) { void f2(void); int x; return &x; } /* expect: warning return-local-addr */
int (*r13(void))[2] { int a[2][2] = {{0}}; return a; } /* expect: warning return-local-addr */
int (*r14(void))[4] { int a[4] = {0}; return &a; } /* expect: warning return-local-addr */
int *r15(void) { int a[2][2] = {{0}}; return a[1]; } /* expect: warning return-local-addr */
/* Members, subscripts and the folded forms GCC descends through. */
int *s1(void) { int a[4] = {0}; return &a[2]; } /* expect: warning return-local-addr */
int *s2(int k) { int a[4] = {0}; return &a[k]; } /* expect: warning return-local-addr */
int *s3(void) { struct S s = {0}; return &s.a[1]; } /* expect: warning return-local-addr */
int *s4(void) { struct S s[2] = {0}; return &s[1].a[2]; } /* expect: warning return-local-addr */
int *s5(void) { int a[4] = {0}; return &2[a]; } /* expect: warning return-local-addr */
int *s6(void) { struct B s = {0}; return s.a; } /* expect: warning return-local-addr */
int *s7(void) { struct S s = {0}; return &(*&s).m; } /* expect: warning return-local-addr */
int *s8(void) { struct { int m; } s = {0}; return &(&s)->m; } /* expect: warning return-local-addr */
int *s9(void) { int x = 0; return &*&x; } /* expect: warning return-local-addr */
int *s10(void) { int x = 0; return &(x); } /* expect: warning return-local-addr */
int *s11(void) { _Complex double z = 0; return (int *)&__real__ z; } /* expect: warning return-local-addr */
/* Offsets and conversions GCC strips. */
int *t1(void) { int a[4] = {0}; return a + 1; } /* expect: warning return-local-addr */
int *t2(void) { int a[4] = {0}; return 1 + a; } /* expect: warning return-local-addr */
int *t3(void) { int a[4] = {0}; return a + i; } /* expect: warning return-local-addr */
int *t4(void) { int a[4] = {0}; return i + a; } /* expect: warning return-local-addr */
int *t5(void) { int a[4] = {0}; return a - 0; } /* expect: warning return-local-addr */
int *t6(void) { int x = 0; return &x - 0; } /* expect: warning return-local-addr */
int *t7(void) { int x = 0; return (&x) + 0; } /* expect: warning return-local-addr */
int *t8(void) { int x = 0; return 1 + &x - 1; } /* expect: warning return-local-addr */
int *t9(void) { int x = 0; return &x + i; } /* expect: warning return-local-addr */
int *t10(void) { int a[4] = {0}; int *q = a; return a - (q - a); } /* expect: warning return-local-addr */
int *t11(void) { int a[4] = {0}; return &a[0] - (&a[1] - &a[0]); } /* expect: warning return-local-addr */
char *t12(void) { int x = 0; return (char *)&x; } /* expect: warning return-local-addr */
int *t13(void) { struct S s = {0}; return (int *)&s; } /* expect: warning return-local-addr */
int *t14(void) { int x = 0; return (int *)(void *)&x; } /* expect: warning return-local-addr */
int *t15(void) { int a[4] = {0}; return (int *)((char *)a + 4); } /* expect: warning return-local-addr */
int *t16(void) { int x; return (int *)((char *)&x - 1); } /* expect: warning return-local-addr */
int *t17(void) { int x; return (int *)((char *)&x - (char *)0); } /* expect: warning return-local-addr */
int *t18(void) { int x = 0; return (int *)(long)&x; } /* expect: warning return-local-addr */
int *t19(void) { int x = 0; return (int *)(int)(long)&x; } /* expect: warning return-local-addr */
int *t20(void) { int x; return (int *)(long)(int *)(char *)&x; } /* expect: warning return-local-addr */
int *t21(void) { int x = 0; return (int *)(4 + (long)&x); } /* expect: warning return-local-addr */
int *t22(void) { int x = 0; return (int *)(0 + (long)&x); } /* expect: warning return-local-addr */
int *t23(void) { int x = 0; return (int *)(L + (long)&x); }
int *t24(void) { int x = 0; return (int *)((long)&x + L); }
int *t25(void) { int x = 0, y = 0; return (int *)((long)&x - (long)&y); }
int *t26(void) { int x = 0; return (int *)(_Bool)&x; }
int *t27(void) { int x; return (int *)~(long)&x; }
int *t28(void) { int x; return (int *)-(long)&x; }
int *t29(void) { int x; return (int *)(&x == 0); }
/* Folds: commas without side effects, conditions of known truth, identical
 * arms, _Generic and __builtin_choose_expr. */
int *u1(void) { int x = 0; return (0, &x); } /* expect: warning return-local-addr */
int *u2(void) { int x = 0; return (i, &x); } /* expect: warning return-local-addr */
int *u3(void) { int x = 0; return (void)0, &x; } /* expect: warning return-local-addr */
int *u4(void) { int x = 0; return (g, i, &x); } /* expect: warning return-local-addr */
int *u5(void) { int x = 0; return ((void)i, &x); } /* expect: warning return-local-addr */
int *u6(void) { int x = 0; return (f(), &x); }
int *u7(void) { int x = 0; return (i++, &x); }
int *u8(void) { int x = 0; return 0 ? &g : &x; } /* expect: warning return-local-addr */
int *u9(void) { int x = 0; return 1 ? &x : &g; } /* expect: warning return-local-addr */
int *u10(int c) { int x = 0; return c ? &x : &x; } /* expect: warning return-local-addr */
int *u11(void) { int x = 0; return (2 > 1) ? &x : 0; } /* expect: warning return-local-addr */
int *u12(void) { int x = 0; return sizeof(int) == 4 ? &x : 0; } /* expect: warning return-local-addr */
int *u13(void) { int x = 0; return &g ? &x : 0; } /* expect: warning return-local-addr */
int *u14(void) { int x = 0; return &w ? &x : 0; }
int *u15(void) { int x = 0; int a[2]; return a ? &x : 0; } /* expect: warning return-local-addr */
int *u16(void) { int x = 0; return "s" ? &x : 0; } /* expect: warning return-local-addr */
int *u17(void) { int x = 0; return (f(), 0) ? 0 : &x; }
int *u18(void) { int x = 0; return &x ?: &g; } /* expect: warning return-local-addr */
int *u19(void) { int x = 0; return 0 ?: &x; } /* expect: warning return-local-addr */
int *u20(int c) { int x = 0; return c ? &x : &g; }
int *u21(int c) { int x = 0, y = 0; return c ? &x : &y; }
int *u22(void) { int x = 0; return _Generic(0, int: &x, default: &g); } /* expect: warning return-local-addr */
int *u23(void) { int x = 0; return __builtin_choose_expr(1, &x, &g); } /* expect: warning return-local-addr */
int *u24(void) { int x = 0; return __builtin_choose_expr(0, &x, &g); }
/* Labels. */
void *l1(void) { lab: return &&lab; } /* expect: warning return-local-addr */
void *l2(void) { l: return (char *)&&l + 1; } /* expect: warning return-local-addr */
void *l3(void) { l: return 0 ? 0 : &&l; } /* expect: warning return-local-addr */
long l4(void) { l: return (long)&&l; }
/* Silent: storage that outlives the call, pointer dereferences, other
 * returns, value flow and non-pointer returns. */
int *n1(void) { static int s; return &s; }
int *n2(void) { extern int e; return &e; }
int *n3(void) { static _Thread_local int t; return &t; }
int *n4(void) { register int *p = &g; return p; }
int *n5(void) { int x = 0; int *p = &x; return p; }
int *n6(void) { return (int[]){1, 2}; }
int *n7(int a[4]) { return a; }
int *n8(int *p) { return &p[0]; }
int *n9(void) { struct S s = {0}, *ps = &s; return &ps->m; }
int *n10(void) { int x = 0; int *p = &x; return &*p; }
struct S *n11(void) { struct S s = {0}; return s.n; }
int *n12(void) { return ({ int y = 0; &y; }); }
long n13(void) { int x = 0; return (long)&x; }
_Bool n14(void) { int x = 0; return &x; }
int *n15(void) { int x; return __builtin_assume_aligned(&x, 4); }
void *n16(void) { char b[8]; return __builtin_memcpy(b, "x", 2); }
int *n17(void) { int x; int *n(void) { return &x; } return n(); }
int *n18(void) { char *s = "lit"; return (int *)s; }
