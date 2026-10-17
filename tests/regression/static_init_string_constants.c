/* String and function-name address constants in static initializers, and
 * __builtin_constant_p on them; every value is GCC 16.2's (-O0 and -O2).
 *
 * lccc stored `.f = 1 ? "x" : "y"` in a struct field, and `0 ? "c" : "d"` in a
 * pointer array, as a NULL pointer without a diagnostic: the initializer
 * classifiers did not look inside `?:` and the byte-buffer path wrote 0 for
 * any element it could not evaluate. The kernel's dynamic-debug descriptors
 * (`.format = __builtin_constant_p(MOD ": ...") ? MOD ": ..." : ...`) and
 * tracepoint strings (`static const char *s = __func__;`) are the shapes.
 */
#include <stdio.h>

struct t { const char *f; unsigned n; };
struct u { const char *f; };
static const struct t s1 = { .f = 1 ? "s1" : "no", .n = 1 };
static const struct t s2 = { 0 ? "no" : "s2", 2 };
static const struct u s3 = { (1 ? "s3" : "no") };
static const char *arr[] = { 1 ? "a0" : "no", 0 ? "no" : "a1" };
static const char *p1 = 1 ? "p1" : "no";
static const struct t dd = {
    .f = __builtin_constant_p("bm" ": register: ") ? "bm" ": register: " : "bm",
    .n = 4,
};

int gv;
int ga[4];
void fn(void) {}

/* GCC's fold_builtin_constant_p: 1 for a string literal's address through
 * no-op conversions (pointer casts, pointer-width integers), `&"x"`, and a
 * constant-condition `?:`; 0 for pointer arithmetic, dereferences, objects,
 * functions, __func__ and the comma operator. */
#define LIST                                                                   \
    X("x") X(("x")) X("x" + 1) X((const char *)"x") X((void *)"x") X(&"x")     \
    X((long)"x") X(&gv) X(ga) X(fn) X(&ga[1]) X(sizeof "x") X(1 ? "a" : "b")   \
    X(("a", "b")) X((char)(long)"x")
#define X(e) __builtin_constant_p(e),
static const int in_init[] = { LIST };

static const char *name(void) { static const char *s = __func__; return s; }
static const char *name_tail(void) { static const char *s = __func__ + 4; return s; }
static const char *pretty(void) { static const char *s = __PRETTY_FUNCTION__; return s; }
static int same_object(void) { static const char *s = __func__; return s == __func__; }

int main(void)
{
    const int in_code[] = { LIST };
    printf("%s %s %s %s %s %s %u\n", s1.f, s2.f, s3.f, arr[0], arr[1], p1, s1.n + s2.n);
    printf("%s %u\n", dd.f, dd.n);
    for (unsigned i = 0; i < sizeof in_init / sizeof *in_init; i++)
        printf("%d%d ", in_init[i], in_code[i]);
    printf("\n%s [%s] %s %d\n", name(), name_tail(), pretty(), same_object());
    return 0;
}
