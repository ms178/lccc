/* The parser pre-filters nested-function speculation with a pure token scan
 * (could_be_nested_function_def): plain typed declarations must skip the
 * eleven-snapshot speculative parse, while every true nested-definition
 * shape must still reach it.  These are the adversarial shapes for that
 * filter: K&R heads whose parameter declarations put `,` and `;` at
 * brace-depth zero before the body, a nested definition returning a struct
 * defined inline (a top-level `{` that is NOT the function body), and the
 * ordinary-declaration shapes that must keep falling through (function
 * pointers, multi-declarator lists, braced initialisers, VLAs). */
#include <stdio.h>

int use_fp(int (*fn)(int), int v) { return fn(v); }

int main(void) {
    /* K&R head with a multi-identifier parameter declaration: the `,` in
     * `int a, b;` sits at depth 0 between the parameter list and the body. */
    int f(a, b) int a, b; { return a * 10 + b; }

    /* Implicit-int K&R shape: `;` at depth 0 before the body. */
    int g(a) int a; { return a + 1; }

    /* Inline tag definition in the return type: the scan must skip the
     * struct body and still find the real function body. */
    struct S { int v; } mk(void) { struct S s; s.v = 7; return s; }

    /* Attribute before the declarator (the GCC-portable spelling). */
    __attribute__((noinline)) int h(int n) { return n * 2; }

    /* Capture check: the filter must not break chain materialisation. */
    int captured = 100;
    int add_captured(int n) { return n + captured; }

    /* Ordinary declarations that must NOT be mistaken for definitions. */
    int (*fp)(int) = g;               /* function-pointer initialiser */
    int x = 5, y[] = { 1, 2 };        /* multi-declarator + braced init */
    int vla_n = 3;
    int vla[vla_n];                   /* VLA: brackets, no parens */
    for (int i = 0; i < vla_n; i++)
        vla[i] = i;

    printf("%d %d %d %d %d %d %d %d\n",
           f(3, 4),                   /* 34 */
           g(9),                      /* 10 */
           mk().v,                    /* 7 */
           h(21),                     /* 42 */
           add_captured(11),          /* 111 */
           use_fp(fp, 1),             /* 2 */
           x + y[1],                  /* 7 */
           vla[2]);                   /* 2 */
    return 0;
}
