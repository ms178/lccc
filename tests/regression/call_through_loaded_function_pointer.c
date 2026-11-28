/* The type of a call through a function pointer that is LOADED.
 *
 * `(*f)(x)` of a function pointer `f` is `f(x)`: the star designates the
 * function `f` points to, which is `f` again, as often as it is written.
 * `(*pp)(x)` of a pointer TO a function pointer is something else: the star
 * loads the function pointer, and the type of the call is the return type of
 * what was loaded.
 *
 * The call itself always was lowered that way.  The questions about its type
 * were not: they took every star off the callee and typed the call by what was
 * left, so `(*pp)(x)` with `int (**pp)(int)` was typed by `pp` -- one level of
 * pointer was taken for the function pointer and the next for its return
 * type.  An `int` came back as a pointer, which is 64 bits wide, unsigned and
 * no floating type:
 *
 *   long f(void)   { return (*pp)(1); }      4294967295 for -1
 *   int  g(void)   { return (*pp)(1) < 0; }  0 for 1
 *   double h(void) { return (*dpp)(5.0) + 1; }
 *                                            1 added to the bits of the double
 *
 * and with two levels, `(**ppp)(x)`, the program was refused: "returning
 * 'double (**)(double)' from a function with return type 'double'".
 *
 * Every function below is called through each way a function pointer can be
 * loaded -- a parameter, a global, a local, a member, an array element named by
 * `*`, a cast, two and three levels -- with a result that is narrower than a
 * register, signed, or no integer at all, and the result is used where its
 * type decides the value: widened, compared with zero, added to a double.
 * `sizeof` and `_Generic` ask for the type itself.
 *
 * The stdout of this test must be byte-identical to gcc-16's; see
 * tests/regression/run_regression.py for the protocol.
 */
#include <stdio.h>

struct pair { long a, b; };

static int minus(int x) { return -x; }
static short sminus(short x) { return (short)-x; }
static signed char cminus(signed char x) { return (signed char)-x; }
static unsigned char ubyte(unsigned char x) { return (unsigned char)(x + 200); }
static unsigned int uint_max(int x) { return 0xffffffffu - (unsigned)x; }
static float fhalf(float x) { return x / 2; }
static double dhalf(double x) { return x / 2; }
static long long wide(int x) { return 0x100000000LL + x; }
static struct pair mk(long x) { struct pair p = { x, -x }; return p; }
static _Bool truth(int x) { return x != 0; }
static int *self(int *p) { return p; }

typedef int (*int_fn)(int);
static int_fn pick(int which) { return which ? minus : (int_fn)0; }

int (*g_fn)(int) = minus;
int (**g_pp)(int) = &g_fn;
int (***g_ppp)(int) = &g_pp;
double (*g_dfn)(double) = dhalf;
double (**g_dpp)(double) = &g_dfn;
int (*g_tab[2])(int) = { minus, minus };
short (*g_stab[1])(short) = { sminus };

struct box {
    int (**pp)(int);
    int (*tab[2])(int);
    double (**dpp)(double);
    int_fn (**picker)(int);
};

#define NI __attribute__((noinline))

/* A parameter that points to a function pointer. */
NI long p_int(int (**pp)(int)) { return (*pp)(1); }
NI int p_int_neg(int (**pp)(int)) { return (*pp)(1) < 0; }
NI double p_int_div(int (**pp)(int)) { return (*pp)(5) / 2.0; }
NI long p_int_add(int (**pp)(int), long y) { return y + (*pp)(7); }
NI unsigned long p_int_size(int (**pp)(int)) { return sizeof((*pp)(5)); }
NI int p_int_generic(int (**pp)(int)) { return _Generic((*pp)(5), int: 1, long: 2, default: 3); }
NI long p_short(short (**pp)(short)) { return (*pp)(3); }
NI int p_short_neg(short (**pp)(short)) { return (*pp)(3) < 0; }
NI long p_schar(signed char (**pp)(signed char)) { return (*pp)(5); }
NI long p_ubyte(unsigned char (**pp)(unsigned char)) { return (*pp)(100) + 1L; }
NI long p_uint(unsigned int (**pp)(int)) { return (*pp)(0); }
NI double p_float(float (**pp)(float)) { return (*pp)(5.0f) + 1; }
NI double p_double(double (**pp)(double)) { return (*pp)(5.0) + 1; }
NI int p_double_generic(double (**pp)(double)) { return _Generic((*pp)(5.0), double: 1, default: 0); }
NI long long p_wide(long long (**pp)(int)) { return (*pp)(3); }
NI long p_pair(struct pair (**pp)(long)) { struct pair p = (*pp)(7); return p.a * 100 + p.b; }
NI long p_pair_member(struct pair (**pp)(long)) { return (*pp)(9).b; }
NI int p_bool(_Bool (**pp)(int)) { return (*pp)(4) + 1; }
NI int p_pointer(int *(**pp)(int *), int *q) { return *(*pp)(q) + 1; }

/* Two and three levels: each star but the last one written is a load. */
NI long p2_int(int (***ppp)(int)) { return (**ppp)(1); }
NI double p2_double(double (***ppp)(double)) { return (**ppp)(9.0) + 1; }
NI long p3_int(int (****pppp)(int)) { return (***pppp)(1); }
/* A star too many designates the function the loaded pointer points to. */
NI long p_extra_star(int (**pp)(int)) { return (**pp)(1); }
NI long p_extra_stars(int (**pp)(int)) { return (****pp)(1); }

/* Globals, a local, and the first element of an array named by a star. */
NI long g_int(void) { return (*g_pp)(1); }
NI long g2_int(void) { return (**g_ppp)(1); }
NI double g_double(void) { return (*g_dpp)(5.0) + 1; }
NI long g_first(void) { return (*g_tab)(1); }
NI long g_first_short(void) { return (*g_stab)(3); }
NI long g_second(void) { return (*(g_tab + 1))(1); }
NI long l_int(void) {
    int (*f)(int) = minus;
    int (**volatile lpp)(int) = &f;
    return (*lpp)(1);
}
NI long l_array(void) {
    int (*tab[3])(int) = { minus, minus, minus };
    int (**p)(int) = tab;
    long sum = 0;
    for (int i = 0; i < 3; i++)
        sum += (*p++)(i + 1);
    return sum;
}

/* Members, a cast, a conditional, and a call that returns a function pointer. */
NI long m_int(struct box *b) { return (*b->pp)(1); }
NI long m_first(struct box *b) { return (*b->tab)(1); }
NI long m_index(struct box *b) { return b->tab[1](1); }
NI double m_double(struct box *b) { return (*b->dpp)(5.0) + 1; }
NI long c_int(void *p) { return (*(int (**)(int))p)(1); }
NI long q_int(int c, int (**a)(int), int (**b)(int)) { return (*(c ? a : b))(1); }
NI long r_int(struct box *b) { return (*(*b->picker)(1))(1); }
NI long r_direct(void) { return (*pick(1))(1); }

/* The controls: a star on a function pointer or on a function name loads
 * nothing, however many there are. */
NI long f_plain(int (*f)(int)) { return f(1); }
NI long f_star(int (*f)(int)) { return (*f)(1); }
NI long f_stars(int (*f)(int)) { return (***f)(1); }
NI long n_star(void) { return (*minus)(1); }
NI long n_stars(void) { return (**minus)(1); }
NI double fd_star(double (*f)(double)) { return (*f)(5.0) + 1; }

int main(void) {
    int (*fn)(int) = minus;
    int (**pp)(int) = &fn;
    int (***ppp)(int) = &pp;
    short (*sfn)(short) = sminus;
    signed char (*cfn)(signed char) = cminus;
    unsigned char (*ufn)(unsigned char) = ubyte;
    unsigned int (*uifn)(int) = uint_max;
    float (*ffn)(float) = fhalf;
    double (*dfn)(double) = dhalf;
    double (**dpp)(double) = &dfn;
    long long (*wfn)(int) = wide;
    struct pair (*pfn)(long) = mk;
    _Bool (*bfn)(int) = truth;
    int *(*selffn)(int *) = self;
    int_fn (*picker)(int) = pick;
    int forty = 40;
    struct box b = { &fn, { minus, minus }, &dfn, &picker };

    printf("param int      %ld %d %g %ld %lu %d\n", p_int(&fn), p_int_neg(&fn), p_int_div(&fn),
           p_int_add(&fn, 10), p_int_size(&fn), p_int_generic(&fn));
    printf("param narrow   %ld %d %ld %ld %ld\n", p_short(&sfn), p_short_neg(&sfn), p_schar(&cfn),
           p_ubyte(&ufn), p_uint(&uifn));
    printf("param float    %g %g %d\n", p_float(&ffn), p_double(&dfn), p_double_generic(&dfn));
    printf("param other    %lld %ld %ld %d %d\n", p_wide(&wfn), p_pair(&pfn), p_pair_member(&pfn),
           p_bool(&bfn), p_pointer(&selffn, &forty));
    printf("levels         %ld %g %ld %ld %ld\n", p2_int(&pp), p2_double(&dpp), p3_int(&ppp),
           p_extra_star(&fn), p_extra_stars(&fn));
    printf("objects        %ld %ld %g %ld %ld %ld %ld %ld\n", g_int(), g2_int(), g_double(), g_first(),
           g_first_short(), g_second(), l_int(), l_array());
    printf("expressions    %ld %ld %ld %g %ld %ld %ld %ld %ld\n", m_int(&b), m_first(&b), m_index(&b),
           m_double(&b), c_int(&fn), q_int(1, &fn, &fn), q_int(0, &fn, &fn), r_int(&b), r_direct());
    printf("controls       %ld %ld %ld %ld %ld %g\n", f_plain(minus), f_star(minus), f_stars(minus),
           n_star(), n_stars(), fd_star(dhalf));
    return 0;
}
