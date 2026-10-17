/* A variadic always_inline wrapper in glibc's _FORTIFY_SOURCE shape —
 * `__builtin_va_arg_pack_len()` deciding, `__builtin_va_arg_pack()`
 * forwarding — inlined into a call site that forwards one, two, and zero
 * arguments.
 *
 * The inliner expands the two sentinels the lowering emits for those
 * builtins: the `__lccc_va_arg_pack` sentinel is deleted and its argument
 * slot spliced with the call site's forwarded arguments, and the
 * `__lccc_va_arg_pack_len` sentinel becomes an I32 copy of their count.
 * The length sentinel's rewrite matched nothing unless the clone happened
 * to be offset by zero (`Value(dest + offset)` where the clone's dest had
 * already been shifted), so an inlined wrapper kept a live call to the
 * undefined `__lccc_va_arg_pack_len` and the program did not link — this is
 * exactly how gzip 1.14's gnulib `open-safer.c` failed to link under glibc
 * fortify (GCC, Clang and ICX all fold the check away).
 *
 * The count is folded even when nothing is forwarded: `wrap0` checks
 * `__builtin_va_arg_pack_len() != 0` at a call site that forwards nothing,
 * which must decide the branch (the reference compilers fold it to 0).
 * GCC agrees on the whole fixture (exit 0, "OK").
 */
#include <stdarg.h>
#include <stdio.h>

int too_many_calls;
void too_many(void) { too_many_calls++; }

int impl(const char *p, int o, ...) {
    va_list ap;
    va_start(ap, o);
    int m = va_arg(ap, int);
    va_end(ap);
    return o * 100 + m + (p[0] == 'x' ? 0 : 100000);
}

__attribute__((always_inline, gnu_inline)) extern __inline int
wrap(const char *p, int o, ...) {
    if (__builtin_va_arg_pack_len() > 1)
        too_many();
    return impl(p, o, __builtin_va_arg_pack());
}

int impl0(const char *p, int o) { return o + (p[0] == 'x' ? 0 : 100000); }

__attribute__((always_inline, gnu_inline)) extern __inline int
wrap0(const char *p, int o, ...) {
    if (__builtin_va_arg_pack_len() != 0)
        too_many();
    return impl0(p, o);
}

int main(void) {
    int rc = 0;
    if (wrap("x", 7, 3) != 703) {
        printf("FAIL one-forwarded\n");
        rc = 1;
    }
    if (too_many_calls != 0) {
        printf("FAIL too_many ran early (%d)\n", too_many_calls);
        rc = 1;
    }
    /* Two forwarded arguments: the count check must fire (it is what glibc's
     * wrapper uses to reject the invalid call shape) and the first forwarded
     * argument still reaches the callee. */
    if (wrap("x", 5, 1, 2) != 501) {
        printf("FAIL two-forwarded\n");
        rc = 1;
    }
    if (too_many_calls != 1) {
        printf("FAIL too_many not run (%d)\n", too_many_calls);
        rc = 1;
    }
    /* Nothing forwarded: the count is 0, not "unknown". */
    if (wrap0("x", 9) != 9) {
        printf("FAIL zero-forwarded\n");
        rc = 1;
    }
    if (too_many_calls != 1) {
        printf("FAIL zero-forwarded len != 0 (%d)\n", too_many_calls);
        rc = 1;
    }
    if (rc == 0)
        printf("OK\n");
    return rc;
}
