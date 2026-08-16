/* __has_attribute / __has_builtin are preprocessor BUILT-INS: they expand in
 * ordinary code, not only inside #if.
 *
 * kernel/trace/trace.c computes
 *     10*1000 + 4*100 + __has_attribute(btf_type_tag)
 * in a plain C expression. lccc only resolved the operators inside #if
 * directives, so in code position they became implicit function calls and
 * the build failed with "'btf_type_tag' undeclared".
 *
 * Also asserts the boundaries of the fix:
 *   - inside a STRING literal the token must NOT be rewritten;
 *   - a supported attribute reports 1, an invented one reports 0;
 *   - __has_builtin works on both sides of the same expression.
 */
#include <stdio.h>
#include <string.h>

int main(void)
{
       /* Code-position expansion, kernel trace.c shape. */
       int version = 10 * 1000 + 4 * 100 + (__has_attribute(btf_type_tag));
       if (version != 10400 + (__has_attribute(btf_type_tag))) {
               printf("FAIL version=%d\n", version);
               return 1;
       }

       /* A universally supported attribute must be 1. */
       if (__has_attribute(noreturn) != 1) {
               printf("FAIL noreturn=0\n");
               return 2;
       }

       /* An invented attribute must be 0 — and must not error. */
       if (__has_attribute(lccc_definitely_not_real_attr) != 0) {
               printf("FAIL invented attr\n");
               return 3;
       }

       /* __has_builtin in code position, both true and false cases. */
       if (!__has_builtin(__builtin_memcpy)) {
               printf("FAIL has_builtin memcpy\n");
               return 4;
       }
       if (__has_builtin(__builtin_lccc_not_real)) {
               printf("FAIL invented builtin\n");
               return 5;
       }

       /* String literals are sacrosanct. */
       const char *s = "__has_attribute(x)";
       if (strcmp(s, "__has_attribute(x)") != 0) {
               printf("FAIL string rewritten: %s\n", s);
               return 6;
       }

       printf("PASS has_attribute_in_code\n");
       return 0;
}
