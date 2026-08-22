/* glibc hidden_proto: gnu89 extern-inline body reached via __asm__ label.
 *
 * glibc redirects internal calls with
 *   extern __typeof(f) f __asm__("__GI_f") attribute_hidden;
 * while the inline-only body is defined under the C name. The inliner's
 * callee map was keyed by the C name only, so every call site (carrying
 * the __GI_ label after lowering's asm-rename) missed the body: rtld's
 * `__argz_next` / `__option_is_end` / `__option_is_short` survived as
 * undefined externals and the libc.so link failed.
 *
 * The callee map now registers the body under both names (single-call-site
 * accounting sums both spellings).
 */
#include <stdio.h>

extern char *my_next(char *entry) __asm__("__GI_my_next")
    __attribute__((visibility("hidden")));
extern __inline __attribute__((__gnu_inline__)) char *my_next(char *entry)
{
    return entry + 1;
}

__attribute__((noinline)) char *walk2(char *p) { return my_next(my_next(p)); }

int main(void)
{
    char b[4] = "abc";
    int ok = *walk2(b) == 'c' && *my_next(b) == 'b';
    printf("hidden-proto:%s\n", ok ? "ok" : "MISMATCH");
    return ok ? 0 : 1;
}
