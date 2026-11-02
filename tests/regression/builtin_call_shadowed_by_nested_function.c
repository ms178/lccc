/* A GNU nested function definition shadows a library function name inside its
 * enclosing function, and the fold table must not answer for the bare spelling.
 *
 * Baseline on main printed `MISMATCH 3 92 5 3 4`:
 *   - in `nested_strlen_vla`, sema's `sizeof_expr` folded `strlen("abc")` to 3
 *     instead of letting the VLA bound call the nested `strlen` (witness 91);
 *   - in `nested_and_builtin`, the lowerer's `eval_const_expr` folded the
 *     condition `strlen("abc") == 73` to `3 == 73` (0) instead of calling the
 *     nested `strlen` (witness 73), while the `__builtin_strlen("abcd")` row
 *     alongside it rightly folded to 4.
 * After the fix: `ok 91 92 5 73 4`.
 *
 * The two spellings must differ, and gcc 14.2 is the oracle for both:
 *   - bare `strlen(...)` inside the scope calls the nested function;
 *   - `__builtin_strlen(...)` is the compiler's own interface and still folds,
 *     because a shadowing declaration carries the bare basename -- the guard
 *     keys on the name AS SPELLED, which is what keeps the prefixed form
 *     foldable (and keeps `-fno-builtin` from disabling a compiler interface).
 * The nested-function rows are also the reason sema's rule declines a
 * block-scope function declaration outright: telling a redundant prototype from
 * a nested definition needs a quadratic scan of the mangled `parent.name` keys,
 * and declining both is the answer that matches gcc on every measured cell.
 */
#include <stdio.h>

__attribute__((noinline)) static int nested_strlen_vla(void)
{
    int strlen(const char *s) { (void)s; return 91; }
    char vla[strlen("abc")];
    return (int)sizeof(vla);
}

__attribute__((noinline)) static int nested_abs_row(void)
{
    int abs(int v) { (void)v; return 92; }
    return (abs(-5) == 92 ? 92 : 5) * 100 + __builtin_abs(-5);
}

__attribute__((noinline)) static int nested_and_builtin(void)
{
    int strlen(const char *s) { (void)s; return 73; }
    /* the nested definition wins for the bare spelling in a folded condition ... */
    int bare = (strlen("abc") == 73) ? 73 : 3;
    /* ... and the compiler's own interface still folds to the library answer */
    int prefixed = (int)__builtin_strlen("abcd");
    return bare * 100 + prefixed;
}

int main(void)
{
    int a = nested_strlen_vla();
    int bc = nested_abs_row();
    int b = bc / 100;
    int c = bc % 100;
    int de = nested_and_builtin();
    int d = de / 100;
    int e = de % 100;
    if (a == 91 && b == 92 && c == 5 && d == 73 && e == 4)
        printf("ok %d %d %d %d %d\n", a, b, c, d, e);
    else
        printf("MISMATCH %d %d %d %d %d\n", a, b, c, d, e);
    return 0;
}
