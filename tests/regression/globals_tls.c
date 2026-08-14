/* global/static initialization, common symbols, TLS, and
 * cross-TU-style linkage of globals and functions. */
#include <stdio.h>
#include <string.h>

int g_a = 42;
static int g_static = 7;
const char *g_str = "global-string";
int g_arr[5] = {1, 2, 3, 4, 5};
int g_zero;                      /* BSS */
double g_fp = 3.25;
static __thread int g_tls = 100; /* TLS */

struct G { int x; int y; } g_struct = {10, 20};
int g_partial[8] = {1, 2};       /* partial init -> rest zero */
static int g_uninit;

static int counter(void) {
    static int c = 0;            /* static local persists */
    return ++c;
}

int main(void) {
    if (g_a != 42) return 1;
    if (g_static != 7) return 2;
    if (strcmp(g_str, "global-string") != 0) return 3;
    for (int i = 0; i < 5; i++) if (g_arr[i] != i + 1) return 4;
    if (g_zero != 0) return 5;
    if (g_fp != 3.25) return 6;
    if (g_tls != 100) return 7;
    if (g_struct.x != 10 || g_struct.y != 20) return 8;
    if (g_partial[0] != 1 || g_partial[1] != 2 || g_partial[2] != 0 || g_partial[7] != 0) return 9;
    if (g_uninit != 0) return 10;

    if (counter() != 1) return 11;
    if (counter() != 2) return 12;
    if (counter() != 3) return 13;

    /* TLS write + read */
    g_tls = 555;
    if (g_tls != 555) return 14;

    /* address stability of globals */
    int *pa = &g_a, *pb = &g_a;
    if (pa != pb) return 15;
    *pa = 99;
    if (g_a != 99) return 16;

    /* string literal dedup semantics (pointer equality within TU) */
    const char *s1 = "same-literal";
    const char *s2 = "same-literal";
    if (s1 != s2) return 17;   /* GCC merges; LCCC's dedup should too */

    printf("OK globals_tls\n");
    return 0;
}
