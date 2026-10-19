/* GNU constant builtins and function-name identifiers, compared with GCC 16.2.
 *
 * - __builtin_choose_expr selects on the condition's value as an integer
 *   constant expression: __builtin_constant_p of a parameter is 0 there
 *   (lccc once picked the constant arm after inlining/propagation).
 * - __builtin_LINE/__builtin_FILE/__builtin_FUNCTION: the invocation line of
 *   a macro, `#line`, file scope (""), ICE contexts (array bound).
 * - __func__/__FUNCTION__/__PRETTY_FUNCTION__ are char[len + 1] arrays
 *   (sizeof was 4/8), "" / "top level" at file scope (lowering panicked),
 *   and a GNU nested function's own name (lccc printed "outer.inner").
 * - __builtin_classify_type of a vector is 19, __has_builtin(__builtin_LINE).
 * - A block-scope enumerator shadows and is restored at block end (the
 *   parser's _Static_assert evaluator once leaked it to file scope).
 */
#include <stdio.h>
#include <string.h>

static const char *base(const char *path)
{
    const char *slash = strrchr(path, '/');
    return slash ? slash + 1 : path;
}

/* choose_expr: no propagation into the condition. */
static int s1;
__attribute__((noinline)) int choose_param(int x)
{
    return __builtin_choose_expr(__builtin_constant_p(x), 100, x);
}
static inline int choose_inline(int x)
{
    return __builtin_choose_expr(__builtin_constant_p(x), 100, x);
}
int choose_local(int x)
{
    int y = x;
    return __builtin_choose_expr(__builtin_constant_p(y), 100, y)
        + __builtin_choose_expr(__builtin_constant_p(s1), 1000, 7);
}

/* LINE/FILE/FUNCTION. */
#define WHERE()                                                              \
    printf("%d %s %s %zu\n", __builtin_LINE(), base(__builtin_FILE()),       \
           __builtin_FUNCTION(), sizeof(__builtin_FUNCTION()))
const char *g_function = __builtin_FUNCTION();
int g_line = __builtin_LINE();
static int g_line_array[__builtin_LINE()];

int where(void)
{
    WHERE();
#line 1000 "renamed.c"
    WHERE(
    );
    return __builtin_LINE();
}
#line 60 "gnu_constant_builtins.c"

/* Function names at file scope and in a static local initializer. */
const char *file_func = __func__;
static const char *file_function = __FUNCTION__;
static const char *file_pretty = __PRETTY_FUNCTION__;

int sizes(void)
{
    return sizeof(__func__) + 100 * sizeof(__FUNCTION__);
}

/* GNU nested function: __func__ is the nested function's own name. */
int outer(int x)
{
    int inner(int y)
    {
        printf("%s %zu %s\n", __func__, sizeof(__func__), __builtin_FUNCTION());
        return y + x;
    }
    static const char *mine = __func__;
    printf("%s %zu %s\n", __func__, sizeof __func__, mine);
    return inner(1);
}

/* Enumerator scope. */
enum { A = 1, B = 2 };
int enum_scope(int B)
{
    _Static_assert(A == 1, "a");
    {
        enum { A = 7 };
        _Static_assert(A == 7, "inner");
    }
    _Static_assert(A == 1, "restored");
    return B;
}
int enum_shadowed_by_object(void)
{
    int A = 3;
    {
        _Static_assert(B == 2, "b");
    }
    return A;
}
_Static_assert(A == 1 && B == 2, "file");

typedef int v4si __attribute__((vector_size(16)));
struct S { int a; };
union U { int a; };
enum E { E0 };
int fn(void) { return 0; }

int main(void)
{
    char c = 0;
    _Bool b = 0;
    enum E e = E0;
    int arr[3];
    struct S s;
    union U u;
    v4si v;
    double d;
    _Complex double z = 0;
    void *p = 0;
    float f;

    printf("%d %d %d\n", choose_param(5), choose_inline(6), choose_local(9));
    printf("[%s] %d %zu %d\n", g_function, g_line, sizeof g_line_array,
           __has_builtin(__builtin_LINE));
    int last = where();
    printf("%d [%s][%s][%s] %d\n", last, file_func, file_function, file_pretty,
           sizes());
    printf("%zu %zu %zu | %d %d %d %d %d %d %d %d %d %d %d %d %d | %d %d %d\n",
           sizeof(__func__), sizeof(__FUNCTION__), sizeof(__PRETTY_FUNCTION__),
           __builtin_classify_type(c), __builtin_classify_type(b),
           __builtin_classify_type(e), __builtin_classify_type(arr),
           __builtin_classify_type(s), __builtin_classify_type(u),
           __builtin_classify_type(v), __builtin_classify_type(d),
           __builtin_classify_type(z), __builtin_classify_type(p),
           __builtin_classify_type(fn), __builtin_classify_type(f),
           __builtin_classify_type("ab"), __has_builtin(__builtin_LINE),
           __has_builtin(__builtin_FILE), __has_builtin(__builtin_FUNCTION));
    int nested = outer(2);
    printf("%d %d %d\n", nested, enum_scope(5), enum_shadowed_by_object());
    /* Self-checks independent of the oracle comparison. */
    if (choose_param(5) != 5 || choose_inline(6) != 6 || choose_local(9) != 16)
        return 1;
    if (sizes() != 6 + 100 * 6 || nested != 3 || strcmp(file_pretty, "top level"))
        return 2;
    if (__builtin_classify_type(v) != 19 || sizeof(__func__) != 5)
        return 3;
    return 0;
}
