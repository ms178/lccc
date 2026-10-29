/* Qualifier rules of C23 6.5.16.1p1 for implicit pointer conversions
 * (initialization, assignment, argument passing, return), GCC 16.2's
 * diagnostics. Each line that GCC diagnoses ends in an `expect:` comment
 * naming the severity and the option; every other line must stay silent.
 * tests/regression/check_qualifier_conversions.py checks LCCC against the
 * comments and, when gcc-16 is available, the comments against GCC. */
typedef const char cchar;
typedef const char *cstr;
typedef char *str;
typedef volatile int vint;
struct rec { const char *name; char *const *argv; int n; };

void take(char *);
void take_pp(char **);
void take_cpp(const char **);
void take_v(void *);
void take_cv(const void *);
char *ret_plain(const char *p) { return p; } /* expect: warning discarded-qualifiers */
const char *ret_ok(char *p) { return p; }

void conversions(const char *c, volatile int *v, const volatile int *cv,
                 char *p, char **pp, const char **cpp, char *const *pcp,
                 int *restrict *rpp, const int (*car)[3], struct rec *r,
                 cchar **tpp, cstr *spp, vint *vp, int cond) {
    /* Dropping a qualifier of the pointed-to type: a warning. */
    char *a1 = c; /* expect: warning discarded-qualifiers */
    char *a2 = (const char *)p; /* expect: warning discarded-qualifiers */
    int *a3 = v; /* expect: warning discarded-qualifiers */
    int *a4 = cv; /* expect: warning discarded-qualifiers */
    void *a5 = c; /* expect: warning discarded-qualifiers */
    char **a6 = pcp; /* expect: warning discarded-qualifiers */
    int **a7 = rpp; /* expect: warning discarded-qualifiers */
    int (*a8)[3] = car; /* expect: warning discarded-array-qualifiers */
    char *a9 = r->name; /* expect: warning discarded-qualifiers */
    char *a10 = *tpp; /* expect: warning discarded-qualifiers */
    char *a11 = *spp; /* expect: warning discarded-qualifiers */
    int *a12 = vp; /* expect: warning discarded-qualifiers */
    char *a13;
    a13 = c; /* expect: warning discarded-qualifiers */
    take(c); /* expect: warning discarded-qualifiers */
    take_v(cv); /* expect: warning discarded-qualifiers */
    take_pp(pcp); /* expect: warning discarded-qualifiers */
    char *a14 = cond ? c : p; /* expect: warning discarded-qualifiers */

    /* A qualifier mismatch below the first level: incompatible types. */
    const char **b1 = pp; /* expect: error incompatible-pointer-types */
    char **b2 = cpp; /* expect: error incompatible-pointer-types */
    const char *const *b3 = pp; /* expect: error incompatible-pointer-types */
    volatile int **b4 = (int **)0 + cond; /* expect: error incompatible-pointer-types */
    take_cpp(pp); /* expect: error incompatible-pointer-types */
    cstr *b5 = pp; /* expect: error incompatible-pointer-types */
    char **b6 = spp; /* expect: error incompatible-pointer-types */
    char ***b7 = &cpp; /* expect: error incompatible-pointer-types */

    /* Adding qualifiers, or matching them exactly: silent. */
    const char *ok1 = p;
    const volatile int *ok2 = v;
    const void *ok3 = c;
    char *const *ok4 = pp;
    const char **ok5 = cpp;
    const char **ok6 = tpp;
    cchar **ok7 = cpp;
    const char **ok8 = spp;
    str *ok9 = pp;
    char *const *ok10 = r->argv;
    const char *ok11 = cond ? c : p;
    char *ok12 = "literal";
    void *ok13 = p;
    const int (*ok14)[3] = car;
    volatile int *ok15 = vp;
    take_cv(c);
    take_cpp(cpp);
    take((char *)c);
    (void)a1; (void)a2; (void)a3; (void)a4; (void)a5; (void)a6; (void)a7;
    (void)a8; (void)a9; (void)a10; (void)a11; (void)a12; (void)a13;
    (void)a14; (void)b1; (void)b2; (void)b3; (void)b4; (void)b5; (void)b6;
    (void)b7; (void)ok1; (void)ok2; (void)ok3; (void)ok4; (void)ok5;
    (void)ok6; (void)ok7; (void)ok8; (void)ok9; (void)ok10; (void)ok11;
    (void)ok12; (void)ok13; (void)ok14; (void)ok15;
}

/* Objects whose own qualifiers are exactly known: `&x` and a named array's
 * decay point at them. Struct fields whose simple declarator the parser
 * folds keep their pointee qualifiers. */
struct box { const char *label; const char *names[2]; char **argv; };
static const int g_lim = 3;
static const char g_name[] = "n";
typedef const int cint3[3];
char *ret_cond(int cond, const char *c) { return cond ? c : 0; } /* expect: warning discarded-qualifiers */
char *ret_sum(const char *c) { return c + 1; } /* expect: warning discarded-qualifiers */
void objects(int cond, const int ci, volatile int vi, struct box *bx, const char *c,
             const char **cpp, struct box bv) {
    const int la[3] = { 0 };
    const volatile int lcv = 0;
    cint3 ta = { 0 };
    int plain = 0, pa[2] = { 0 };
    int *o1 = &ci; /* expect: warning discarded-qualifiers */
    int *o2 = &g_lim; /* expect: warning discarded-qualifiers */
    char *o3 = g_name; /* expect: warning discarded-qualifiers */
    int *o4 = la; /* expect: warning discarded-qualifiers */
    int (*o5)[3] = &la; /* expect: warning discarded-array-qualifiers */
    volatile int *o6 = &lcv; /* expect: warning discarded-qualifiers */
    int *o7 = &vi; /* expect: warning discarded-qualifiers */
    int *o8 = ta; /* expect: warning discarded-qualifiers */
    char *o9 = bx->label; /* expect: warning discarded-qualifiers */
    char *o10 = bv.names[cond]; /* expect: warning discarded-qualifiers */
    char **o11 = bx->names; /* expect: error incompatible-pointer-types */
    const char **o12 = bx->argv; /* expect: error incompatible-pointer-types */
    char *o13 = (c); /* expect: warning discarded-qualifiers */
    char *o14 = c + 1; /* expect: warning discarded-qualifiers */
    char *o15 = cpp[cond]; /* expect: warning discarded-qualifiers */
    char **o16 = cpp + cond; /* expect: error incompatible-pointer-types */
    take(cond ? c : 0); /* expect: warning discarded-qualifiers */
    take(c + 1); /* expect: warning discarded-qualifiers */
    char *o17 = (cond); /* expect: error int-conversion */
    const int *ok1 = &ci;
    const volatile int *ok2 = &lcv;
    int *ok3 = &plain;
    int *ok4 = pa;
    const char *const *ok5 = bx->names;
    const int (*ok6)[3] = &ta;
    (void)o1; (void)o2; (void)o3; (void)o4; (void)o5; (void)o6; (void)o7;
    (void)o8; (void)o9; (void)o10; (void)o11; (void)o12; (void)o13; (void)o14;
    (void)o15; (void)o16; (void)o17; (void)ok1; (void)ok2; (void)ok3; (void)ok4;
    (void)ok5; (void)ok6;
}

/* `?:` with a pointer arm, pointer comparisons, transparent unions, and
 * signedness below the pointee. */
typedef void (*thunk)(void);
typedef union { char *s; int *i; } __attribute__((transparent_union)) tunion;
void take_tu(tunion);
void arms_and_comparisons(int cond, int *ip, const int *cip, char *p, void *v, const void *cv,
                          long l, double d, _Bool b, thunk fp, int (*a3)[3], int (*a4)[4],
                          unsigned *up, int **ipp, const int **cipp, const char *c,
                          const char **cpp, char **pp, struct rec s) {
    (void)(cond ? ip : 1); /* expect: error int-conversion */
    (void)(cond ? 1L : ip); /* expect: error int-conversion */
    (void)(cond ? ip : b); /* expect: error int-conversion */
    (void)(cond ? ip : d); /* expect: error none */
    (void)(cond ?: ip); /* expect: error int-conversion */
    (void)(cond ? s : 1); /* expect: error none */
    (void)(cond ? pp : cpp); /* expect: error incompatible-pointer-types */
    (void)(ip == p); /* expect: warning compare-distinct-pointer-types */
    (void)(ip >= p); /* expect: warning compare-distinct-pointer-types */
    (void)(ip < v); /* expect: warning compare-distinct-pointer-types */
    (void)(fp == ip); /* expect: warning compare-distinct-pointer-types */
    (void)(a3 == a4); /* expect: warning compare-distinct-pointer-types */
    (void)(ip == up); /* expect: warning compare-distinct-pointer-types */
    (void)(ipp == cipp); /* expect: warning compare-distinct-pointer-types */
    (void)(pp != cpp); /* expect: warning compare-distinct-pointer-types */
    (void)(ip == (char *)0); /* expect: warning compare-distinct-pointer-types */
    (void)(ip == 1); /* expect: warning none */
    (void)(1 == ip); /* expect: warning none */
    (void)(ip == l); /* expect: warning none */
    take_tu(c); /* expect: warning discarded-qualifiers */
    take_tu(cpp); /* expect: error none */
    unsigned **u2 = ipp; /* expect: error incompatible-pointer-types */
    unsigned char **uc = pp; /* expect: error incompatible-pointer-types */
    unsigned (*ua)[3] = a3; /* expect: error incompatible-pointer-types */
    (void)(cond ? ip : 0);
    (void)(cond ? ip : cip);
    (void)(cond ? fp : v);
    (void)(ip == cip);
    (void)(ip == v);
    (void)(cv == ip);
    (void)(ip == 0);
    (void)(ip < 0);
    (void)(fp == v);
    (void)(fp < fp);
    (void)(ipp == (void *)0);
    take_tu(p);
    take_tu(ip);
    take_tu(0);
    unsigned *us = ip;
    (void)u2; (void)uc; (void)ua; (void)us;
}

/* -Wcompare-distinct-pointer-types is C23 6.2.7 compatibility of the
 * unqualified pointees, not type identity: an enum matches its underlying
 * type (`enum cu` is unsigned int, `enum ci` int), an incomplete array
 * matches a complete one, an unprototyped function type matches a
 * prototype, and a parameter's array type is adjusted. */
enum cu { CU_A };
enum ci { CI_A = -1 };
struct opaque;
void comparisons_by_compatibility(int *ip, unsigned *up, char *p, signed char *sp,
                                  enum cu *eu, enum ci *ei, int (*a3)[3], int (*an)[],
                                  int (*a4)[4], void (*f1)(int), void (*f0)(), void (*f2)(long),
                                  int **ipp, int *const *ipcp, void **vpp, void *v, long *lp,
                                  long long *llp, struct opaque *op, int (*fa)(int[3]),
                                  int (*fb)(int *)) {
    (void)(p == sp); /* expect: warning compare-distinct-pointer-types */
    (void)(eu == ip); /* expect: warning compare-distinct-pointer-types */
    (void)(f1 == f2); /* expect: warning compare-distinct-pointer-types */
    (void)(vpp == ipp); /* expect: warning compare-distinct-pointer-types */
    (void)(lp == llp); /* expect: warning compare-distinct-pointer-types */
    (void)(a3 != an);
    (void)(eu == up);
    (void)(ei == ip);
    (void)(f1 == f0);
    (void)(ipp == ipcp);
    (void)(ipp > ipcp);
    (void)(v == ipp);
    (void)(op == v);
    (void)(fa == fb);
    (void)(a4 == an);
}

/* Casts between pointers and integers of another width (x86-64). */
enum small { SMALL_A };
void casts(int i, short sh, long l, _Bool b, enum small e, void *v, char buf[4]) {
    void *c1 = (void *)i; /* expect: warning int-to-pointer-cast */
    void *c2 = (void *)sh; /* expect: warning int-to-pointer-cast */
    void *c3 = (void *)(i ? 1 : 0); /* expect: warning int-to-pointer-cast */
    int c4 = (int)v; /* expect: warning pointer-to-int-cast */
    char c5 = (char)buf; /* expect: warning pointer-to-int-cast */
    int c6 = (int)(void *)0; /* expect: warning pointer-to-int-cast */
    void *ok1 = (void *)l;
    void *ok2 = (void *)1;
    void *ok3 = (void *)b;
    void *ok4 = (void *)e;
    void *ok5 = (void *)(sizeof(int) + 2);
    long ok6 = (long)v;
    _Bool ok7 = (_Bool)v;
    int ok8 = (int)(long)v;
    (void)c1; (void)c2; (void)c3; (void)c4; (void)c5; (void)c6; (void)ok1; (void)ok2;
    (void)ok3; (void)ok4; (void)ok5; (void)ok6; (void)ok7; (void)ok8;
}

/* An lvalue's own qualifiers: members (with their containing object's, C11
   6.5.2.3p3), pointer objects' own stars, and array parameters adjusted to
   pointers (C23 6.7.6.3p7, including the qualifiers inside `[...]`). */
typedef const int own_ci;
typedef char *const own_cp;
typedef int own_a3[3];
struct own_s {
    const int a[3]; int b[2]; volatile int v; own_ci t; int m; const int c;
    struct { const int x; };
    char *const p; char *volatile pv; char *q; char *const ca[2]; int (*const pa)[3];
};
union own_u { const char s[4]; int i; };
struct own_s own_g;
const struct own_s own_cg;
char *const own_gp = 0;
int *const *own_gpp;
int (*const own_gpa)[3] = 0;
void own_use(int *);
void own_members(struct own_s *ps, const struct own_s *cps, struct own_s sa[2], union own_u *pu,
                 int *q) {
    int *m1 = own_g.a; /* expect: warning discarded-qualifiers */
    int *m2 = &own_g.c; /* expect: warning discarded-qualifiers */
    int *m3 = &own_g.t; /* expect: warning discarded-qualifiers */
    int *m4 = &own_g.v; /* expect: warning discarded-qualifiers */
    int *m5 = &own_cg.m; /* expect: warning discarded-qualifiers */
    int *m6 = own_cg.b; /* expect: warning discarded-qualifiers */
    int *m7 = &cps->m; /* expect: warning discarded-qualifiers */
    int *m8 = ps->a; /* expect: warning discarded-qualifiers */
    int *m9 = &sa[1].c; /* expect: warning discarded-qualifiers */
    int *m10 = &own_g.x; /* expect: warning discarded-qualifiers */
    char *m11 = pu->s; /* expect: warning discarded-qualifiers */
    int *m12 = &(*cps).m; /* expect: warning discarded-qualifiers */
    own_use(&own_cg.m); /* expect: warning discarded-qualifiers */
    int *ok1 = &own_g.m;
    int *ok2 = own_g.b;
    int *ok3 = &ps->m;
    int *ok4 = &q[1];
    const int *ok5 = &own_g.c;
    (void)m1; (void)m2; (void)m3; (void)m4; (void)m5; (void)m6; (void)m7; (void)m8;
    (void)m9; (void)m10; (void)m11; (void)m12; (void)ok1; (void)ok2; (void)ok3; (void)ok4;
    (void)ok5;
}
void own_stars(char *const p, own_cp cp, const own_a3 a, char *r) {
    char **s1 = &own_gp; /* expect: warning discarded-qualifiers */
    char **s2 = &p; /* expect: warning discarded-qualifiers */
    char **s3 = &cp; /* expect: warning discarded-qualifiers */
    char **s4 = &own_g.p; /* expect: warning discarded-qualifiers */
    char **s5 = &own_g.pv; /* expect: warning discarded-qualifiers */
    char **s6 = own_g.ca; /* expect: warning discarded-qualifiers */
    int (**s7)[3] = &own_gpa; /* expect: warning discarded-qualifiers */
    int (**s8)[3] = &own_g.pa; /* expect: warning discarded-qualifiers */
    int ***s9 = &own_gpp; /* expect: error incompatible-pointer-types */
    const int **ok1 = &a;
    char **ok2 = &own_g.q;
    char **ok3 = &r;
    char *const *ok4 = &own_gp;
    (void)s1; (void)s2; (void)s3; (void)s4; (void)s5; (void)s6; (void)s7; (void)s8; (void)s9;
    (void)ok1; (void)ok2; (void)ok3; (void)ok4;
}
void own_arrays(const int a1[2], int *const a2[2], const int a3[2][3], volatile char a4[],
                int a5[const 2], int a6[volatile restrict 2], int a7[2]) {
    int *p1 = a1; /* expect: warning discarded-qualifiers */
    int **p2 = a2; /* expect: warning discarded-qualifiers */
    int (*p3)[3] = a3; /* expect: warning discarded-array-qualifiers */
    char *p4 = a4; /* expect: warning discarded-qualifiers */
    int **p5 = &a5; /* expect: warning discarded-qualifiers */
    int *volatile *p6 = &a6; /* expect: warning discarded-qualifiers */
    a5 = 0; /* expect: error none */
    int *ok1 = a7;
    int **ok2 = &a7;
    a7 = 0;
    (void)p1; (void)p2; (void)p3; (void)p4; (void)p5; (void)p6; (void)ok1; (void)ok2;
}

/* Returns through a declarator that wraps the function in arrays:
 * `int (*f(void))[2]` returns a pointer to an array, whose element
 * qualifiers the base supplies (directly or through an alias). */
const int ret_g2[2][2];
typedef const int ret_ci;
typedef int ret_i;
int (*ret_arr(void))[2] { return ret_g2; } /* expect: warning discarded-array-qualifiers */
ret_i (*ret_alias_plain(void))[2] { return ret_g2; } /* expect: warning discarded-array-qualifiers */
int (*ret_arr_local(void))[2] { static const int l[2][2]; return l; } /* expect: warning discarded-array-qualifiers */
const int (*ret_arr_const(void))[2] { return ret_g2; }
ret_ci (*ret_alias_const(void))[2] { return ret_g2; }
volatile int (*ret_arr_vol(void))[2] { return ret_g2; } /* expect: warning discarded-array-qualifiers */
const int (*const *ret_arr_star(void))[2] { static const int (*const p)[2] = ret_g2; return &p; }
