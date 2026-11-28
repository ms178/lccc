/* A library builtin is called by its own prototype.
 *
 * `__builtin_strchr(s, c)` is `char *strchr(const char *, int)` whether or
 * not <string.h> was included: the argument is converted to the parameter as
 * if by assignment, and the call has the prototype's result type. lccc took
 * the result types of some builtins and the `size_t` parameters of others
 * from hand-written lists, converted between integers only, and gave every
 * builtin no list named an `int` result. With no header in sight (this file
 * includes none), measured against GCC 16.2:
 *
 *   __builtin_sqrt(n), int n = 16     NaN: the int went out in %edi, and
 *                                     sqrt read %xmm0
 *   __builtin_ldexp(3, 2.9)           23.2: the exponent was the bits of 2.9
 *   __builtin_memset(p, 'x', 3.9)     nothing set
 *   __builtin_strchr(s, 98.9)         not found
 *   __builtin_strchr(s, 'b')[1]       read at 8 bytes a step: the result was
 *                                     an `int`, converted to a pointer
 *   __builtin_strlen(s) - 4 < 0       true: the result was signed
 *   __builtin_tan(x)                  an undeclared function, like 233 more
 *
 * The prototypes are GCC's now (`scripts/gen_libc_builtin_types.py`), and a
 * call of a library builtin is lowered as a call of a function declared with
 * that prototype. `builtin_library_arguments.c`, which the same script
 * writes, calls every builtin whose parameters are numbers; this file has
 * the ones that take pointers, the variadic ones, the object-size checking
 * ones, and what the type of a call is. GCC's output is the reference.
 *
 * One defect here was not the builtins': `f(n)` for a function declared
 * `double f(double _Complex)` and an `int n` converted `n` to a complex
 * value and then converted the address of that value from `int` to the
 * parameter, so the call read through a sign-extended half of an address.
 * `conj(n)` and `cacos(2)` with <complex.h> were segmentation faults. */

#define P __builtin_printf

/* The type of an expression by name: `sizeof` alone cannot tell `long` from
 * `unsigned long`, or `char *` from `void *`. */
#define TYPE(e) _Generic((e), \
    int: "int", unsigned: "unsigned", long: "long", unsigned long: "unsigned long", \
    long long: "long long", unsigned long long: "unsigned long long", \
    float: "float", double: "double", long double: "long double", \
    _Complex float: "complex float", _Complex double: "complex double", \
    _Complex long double: "complex long double", \
    char *: "char *", const char *: "const char *", void *: "void *", default: "other")
#define ROW(e) P("%-40s %s\n", #e, TYPE(e))

int n16 = 16, n2 = 2, n3 = 3, neg = -5;
double d29 = 2.9, d39 = 3.9, d989 = 98.9, d25 = 2.5, d05 = 0.5;
float f15 = 1.5f;
long double l35 = 3.5L;
signed char c2 = 2;
unsigned short us7 = 7;
_Complex double zd;
_Complex float zf;
char text[] = "abcdefgh";
const char *cs = "abcde";

static void types(void) {
    char buf[32];
    int e = 0;
    double ip = 0;
    ROW(__builtin_strlen(cs));
    ROW(__builtin_strspn(cs, "ab"));
    ROW(__builtin_strcspn(cs, "d"));
    ROW(__builtin_strnlen(cs, 3));
    ROW(__builtin_strcmp(cs, cs));
    ROW(__builtin_strncmp(cs, cs, 2));
    ROW(__builtin_memcmp(cs, cs, 2));
    ROW(__builtin_strchr(cs, 'a'));
    ROW(__builtin_strrchr(cs, 'a'));
    ROW(__builtin_strstr(cs, "c"));
    ROW(__builtin_strpbrk(cs, "c"));
    ROW(__builtin_strcpy(buf, cs));
    ROW(__builtin_strncpy(buf, cs, 2));
    ROW(__builtin_strcat(buf, cs));
    ROW(__builtin_stpcpy(buf, cs));
    ROW(__builtin_memcpy(buf, cs, 2));
    ROW(__builtin_memmove(buf, cs, 2));
    ROW(__builtin_mempcpy(buf, cs, 2));
    ROW(__builtin_memset(buf, 0, 2));
    ROW(__builtin_memchr(cs, 'b', 4));
    ROW(__builtin_abs(n2));
    ROW(__builtin_labs(n2));
    ROW(__builtin_llabs(n2));
    ROW(__builtin_ffs(n2));
    ROW(__builtin_puts(""));
    ROW(__builtin_snprintf(buf, 4, "a"));
    ROW(__builtin_sqrt(n2));
    ROW(__builtin_sqrtf(n2));
    ROW(__builtin_sqrtl(n2));
    ROW(__builtin_tan(n2));
    ROW(__builtin_atan2f(n2, n3));
    ROW(__builtin_ldexpf(n2, n3));
    ROW(__builtin_frexp(n2, &e));
    ROW(__builtin_modf(n2, &ip));
    ROW(__builtin_ilogb(n2));
    ROW(__builtin_lround(d25));
    ROW(__builtin_llrint(d25));
    ROW(__builtin_isdigit('5'));
    ROW(__builtin_toupper('a'));
    ROW(__builtin_cabs(n2));
    ROW(__builtin_cabsf(n2));
    ROW(__builtin_cexp(n2));
    ROW(__builtin_cexpf(n2));
    ROW(__builtin_cexpl(n2));
    ROW(__builtin_conjf(zd));
    ROW(__builtin_cpow(n2, n3));
    ROW(__builtin_creal(zf));
    ROW(__builtin_cimagf(zd));
    /* The value has the type as well: one step of the pointer is one byte,
     * and the length does not go below zero. */
    P("%c %c %c %ld %d\n", __builtin_strchr(cs, 'b')[1], *__builtin_strchr(cs, 'b'),
      __builtin_strrchr(cs, 'c')[-1], (long)(__builtin_strchr(cs, 'd') - __builtin_strchr(cs, 'b')),
      (int)(__builtin_strlen(cs) - 6 > 0));
    P("%d %d\n", (int)sizeof(__builtin_strlen(cs)), (int)sizeof(*__builtin_strchr(cs, 'b')));
}

/* An argument of another type is converted to the parameter's. */
__attribute__((noinline)) static void strings(void) {
    char d[16] = "aaaaaaaaaaaaaaa";
    char e[16] = "0123456789";
    char f[16];
    /* A length from a double, a character from a double. */
    __builtin_memset(d, 'x', d39);
    P("memset   %s\n", d);
    __builtin_memset(d + 4, d989, n2);
    P("memset   %s\n", d);
    P("strchr   %s\n", __builtin_strchr(cs, d989));
    P("strrchr  %s\n", __builtin_strrchr(cs, d989 + 1));
    P("memchr   %s\n", (char *)__builtin_memchr(cs, d989, d39));
    __builtin_memcpy(e + 1, cs, d29);
    P("memcpy   %s\n", e);
    __builtin_memmove(e + 2, e, d39);
    P("memmove  %s\n", e);
    P("mempcpy  %d\n", (int)((char *)__builtin_mempcpy(f, cs, d39) - f));
    __builtin_strncpy(f, cs, d39);
    f[3] = 0;
    P("strncpy  %s\n", f);
    P("stpncpy  %d\n", (int)(__builtin_stpncpy(f, "xy", d39) - f));
    f[1] = 0;
    __builtin_strncat(f, cs, d29);
    P("strncat  %s\n", f);
    P("strncmp  %d %d\n", __builtin_strncmp("abcx", "abcy", d39) == 0,
      __builtin_strncmp("abcx", "abcy", d39 + 1) < 0);
    P("memcmp   %d %d\n", __builtin_memcmp("abcx", "abcy", d39) == 0,
      __builtin_memcmp("abcx", "abcy", c2 + c2) < 0);
    P("bcmp     %d\n", __builtin_bcmp("abcx", "abcy", d39) == 0);
    P("strncase %d\n", __builtin_strncasecmp("ABcx", "abCy", d39) == 0);
    P("strnlen  %d %d\n", (int)__builtin_strnlen(cs, d29), (int)__builtin_strnlen(cs, f15 * 100));
    /* A negative int is a large size_t, not the low half of a register. */
    P("strnlen  %d\n", (int)__builtin_strnlen(cs, neg));
    __builtin_bzero(e + 1, d29);
    P("bzero    %d%d%d%c\n", e[0] == '0', e[1], e[2], e[3]);
    __builtin_bcopy(cs, e, d39);
    P("bcopy    %c%c%c%c\n", e[0], e[1], e[2], e[3]);
    char *copy = __builtin_strndup(cs, d29);
    P("strndup  %s\n", copy);
    __builtin_free(copy);
    char *more = __builtin_malloc(d39 * 2);
    __builtin_strcpy(more, "malloc");
    more = __builtin_realloc(more, d39 * 8);
    P("realloc  %s\n", __builtin_strcat(more, "+"));
    __builtin_free(more);
    int *zero = __builtin_calloc(d29, d39 + 1);
    P("calloc   %d%d\n", zero[0], zero[1]);
    __builtin_free(zero);
    P("strspn   %d %d\n", (int)__builtin_strspn(cs, "cba"), (int)__builtin_strcspn(cs, "dx"));
    P("strpbrk  %s %s\n", __builtin_strpbrk(cs, "xd"), __builtin_strstr(cs, "bc"));
    P("index    %s %s\n", __builtin_index(cs, d989), __builtin_rindex(cs, d989 + 2));
    P("strcase  %d\n", __builtin_strcasecmp("ABC", "abc"));
}

/* Numbers in and out through pointers; the integer parameters of the
 * floating functions; the integer results. */
__attribute__((noinline)) static void numbers(void) {
    int e = 0, q = 0, sign = 0;
    double ip = 0, s = 0, c = 0;
    float ipf = 0;
    long double ipl = 0;
    P("sqrt     %g %g %g\n", __builtin_sqrt(n16), (double)__builtin_sqrtf(n16),
      (double)__builtin_sqrtl(n16));
    P("pow      %g %g\n", __builtin_pow(n2, 10), __builtin_pow(c2, us7));
    P("ldexp    %g %g %g\n", __builtin_ldexp(n3, d29), (double)__builtin_ldexpf(n3, d29),
      (double)__builtin_ldexpl(n3, d29));
    P("scalbn   %g %g\n", __builtin_scalbn(n3, d29), __builtin_scalbln(n3, d29));
    /* The result first, then what the call stored: the order in which the
     * arguments of one call are evaluated is the compiler's own. */
    double part = __builtin_frexp(n16, &e);
    P("frexp    %g %d\n", part, e);
    part = __builtin_modf(l35, &ip);
    P("modf     %g %g\n", part, ip);
    part = __builtin_modff(d25, &ipf);
    P("modff    %g %g\n", part, (double)ipf);
    part = __builtin_modfl(f15, &ipl);
    P("modfl    %g %g\n", part, (double)ipl);
    part = __builtin_remquo(us7, n3, &q);
    P("remquo   %g %d\n", part, q & 7);
    __builtin_sincos(n2 - n2, &s, &c);
    P("sincos   %g %g\n", s, c);
    part = __builtin_lgamma_r(n3, &sign);
    P("lgamma_r %g %d\n", part, sign);
    P("lround   %ld %lld %ld %lld\n", __builtin_lround(d25), __builtin_llround(f15),
      __builtin_lrint(n3), __builtin_llrint(c2));
    P("ilogb    %d %d\n", __builtin_ilogb(n16), __builtin_ilogbf(us7));
    P("abs      %d %ld %lld\n", __builtin_abs(-d39), __builtin_labs(-d39), __builtin_llabs(-d39));
    P("ffs      %d %d\n", __builtin_ffs(d39 * 2 + 1), __builtin_ffsll(8.9));
    P("jn       %.6f %.6f\n", __builtin_jn(d29, n3), __builtin_yn(d29, n3));
    P("fma      %g %g\n", __builtin_fma(n2, n3, c2), (double)__builtin_fmaf(n2, d25, us7));
    P("copysign %g %g\n", __builtin_copysign(f15, neg), (double)__builtin_copysignf(d25, neg));
    P("fmax     %g %g\n", __builtin_fmax(us7, f15), __builtin_fmin(c2, neg));
    P("nextaft  %d\n", __builtin_nextafter(n2, n3) > 2);
    P("ctype    %d %d %c %c\n", __builtin_isdigit(53.9) != 0, __builtin_isalpha(53.9) != 0,
      __builtin_toupper(97.9), __builtin_tolower(65.9));
    P("wctype   %d %d\n", __builtin_iswalpha(97.9) != 0, (int)__builtin_towupper(97.9));
}

/* A real argument for a complex parameter, a complex one of another width,
 * and a complex result that is used as a number. */
__attribute__((noinline)) static double user_complex(_Complex double z) {
    return __real__ z + 10 * __imag__ z;
}
__attribute__((noinline)) static double user_complex_float(_Complex float z) {
    return __real__ z + 10 * __imag__ z;
}
__attribute__((noinline)) static double user_complex_long(_Complex long double z) {
    return __real__ z + 10 * __imag__ z;
}

__attribute__((noinline)) static void complexes(void) {
    _Bool truth = 1;
    long long wide = 9;
    zd = 1.5 + 0.5i;
    zf = 0.5f + 1.25fi;
    _Complex double r = __builtin_cexp(n2 - n2);
    P("cexp     %g %g\n", __real__ r, __imag__ r);
    r = __builtin_csqrt(neg + 1);
    P("csqrt    %g %g\n", __real__ r, __imag__ r);
    r = __builtin_cpow(n2, d05 * 4);
    P("cpow     %.6f %.6f\n", __real__ r, __imag__ r);
    r = __builtin_cpowf(zd, n2);
    P("cpowf    %.4f %.4f\n", __real__ r, __imag__ r);
    r = __builtin_cpowl(n3, c2);
    P("cpowl    %.6f %.6f\n", __real__ r, __imag__ r);
    r = __builtin_conj(n3);
    P("conj     %g %g\n", __real__ r, __imag__ r);
    r = __builtin_conjf(zd);
    P("conjf    %g %g %d\n", __real__ r, __imag__ r, (int)sizeof(__builtin_conjf(zd)));
    r = __builtin_conjl(zf);
    P("conjl    %g %g %d\n", __real__ r, __imag__ r, (int)(sizeof(__builtin_conjl(zf)) > 16));
    P("cabs     %g %g %g\n", __builtin_cabs(n3), (double)__builtin_cabsf(neg),
      (double)__builtin_cabsl(us7));
    P("carg     %g %d\n", __builtin_carg(n3), __builtin_carg(zf) > 1);
    P("creal    %g %g %g %g\n", __builtin_creal(zf), (double)__builtin_crealf(zd),
      __builtin_cimag(zf), (double)__builtin_cimagf(n3));
    P("cproj    %g\n", __real__ __builtin_cproj(n3));
    /* Every integer narrower than a pointer, for a declared function. */
    P("user     %g %g %g %g %g %g\n", user_complex(n3), user_complex(c2), user_complex(us7),
      user_complex(truth), user_complex(wide), user_complex(f15));
    P("user     %g %g %g %g\n", user_complex_float(n3), user_complex_float(zd),
      user_complex_long(c2), user_complex_long(zf));
}

/* The variadic builtins: the named parameters are converted, the rest is
 * promoted. The checking twins have the object size and a flag besides. */
__attribute__((noinline)) static void formatted(void) {
    char buf[32];
    signed char small = -3;
    short medium = 300;
    int count = __builtin_printf("printf   %g %d %d %c\n", f15, small, medium, 'z');
    P("         %d\n", count);
    count = __builtin_snprintf(buf, d39 * 2, "%s", "0123456789");
    P("snprintf %s %d\n", buf, count);
    count = __builtin_sprintf(buf, "%g/%d", f15, small);
    P("sprintf  %s %d\n", buf, count);
    count = __builtin___snprintf_chk(buf, d39 * 2, n2 - 1.5, sizeof buf, "%s%g", "ab", f15);
    P("snp_chk  %s %d\n", buf, count);
    count = __builtin___sprintf_chk(buf, n2 - 1.5, sizeof buf, "%d-%g", medium, f15);
    P("sp_chk   %s %d\n", buf, count);
    count = __builtin___printf_chk(n2 - 1.5, "pr_chk   %g %d\n", f15, small);
    P("         %d\n", count);
    __builtin___memset_chk(buf, 'm', d39, sizeof buf);
    buf[3] = 0;
    P("mset_chk %s\n", buf);
    __builtin___memcpy_chk(buf, "wxyz", d29 + 2.5, sizeof buf);
    P("mcpy_chk %s\n", buf);
    __builtin___memmove_chk(buf + 1, buf, d39, sizeof buf - 1);
    P("mmov_chk %s\n", buf);
    P("mpcp_chk %d\n", (int)((char *)__builtin___mempcpy_chk(buf, "12", d29, sizeof buf) - buf));
    __builtin___strcpy_chk(buf, "str", sizeof buf);
    __builtin___strcat_chk(buf, "cat", sizeof buf);
    __builtin___strncat_chk(buf, "0123", d29, sizeof buf);
    P("str_chk  %s\n", buf);
    __builtin___strncpy_chk(buf, "zyxwv", d39, sizeof buf);
    P("strn_chk %s %d\n", buf, (int)(__builtin___stpcpy_chk(buf, "end", sizeof buf) - buf));
    P("stpn_chk %d\n", (int)(__builtin___stpncpy_chk(buf, "q", d39, sizeof buf) - buf));
    __builtin_puts("puts");
    __builtin_putchar(d989);
    __builtin_putchar('\n');
}

/* A `va_list` is passed on as it is, between converted arguments. */
__attribute__((noinline)) static int through_list(char *out, double size, const char *format, ...) {
    __builtin_va_list ap;
    __builtin_va_start(ap, format);
    int count = __builtin_vsnprintf(out, size, format, ap);
    __builtin_va_end(ap);
    return count;
}
__attribute__((noinline)) static int through_checked(char *out, double size, const char *format, ...) {
    __builtin_va_list ap;
    __builtin_va_start(ap, format);
    int count = __builtin___vsnprintf_chk(out, size, n2 - 1.5, size * 2, format, ap);
    __builtin_va_end(ap);
    return count;
}
__attribute__((noinline)) static int through_stdout(const char *format, ...) {
    __builtin_va_list ap;
    __builtin_va_start(ap, format);
    int count = __builtin_vprintf(format, ap);
    __builtin_va_end(ap);
    return count;
}

static void lists(void) {
    char buf[32];
    int count = through_list(buf, 8.9, "%d:%g:%s", n3, d25, "abcdef");
    P("vsnprintf %s %d\n", buf, count);
    count = through_checked(buf, 8.9, "%s:%d", "uvwxyz", n16);
    P("vsnp_chk  %s %d\n", buf, count);
    count = through_stdout("vprintf   %g %s\n", d05, "ok");
    P("          %d\n", count);
}

/* A builtin that does not return ends the path: nothing after it is
 * reached, and a function may end there without a value. */
__attribute__((noinline)) static int stops(int x) {
    if (x > 0)
        return x * 2;
    __builtin_abort();
}

int main(void) {
    types();
    strings();
    numbers();
    complexes();
    formatted();
    lists();
    P("stops    %d\n", stops(n3));
    /* The status is an `int`: 0.9 is 0. */
    __builtin_exit(d05 + 0.4);
}
