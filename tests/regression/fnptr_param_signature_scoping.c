/* A function-pointer binding's call signature must follow C identifier
 * scoping (C11 6.2.1): the innermost declaration of the NAME decides, never a
 * same-named declaration from another function, an enclosing block, or a
 * library builtin.
 *
 * Root cause (2026-09-26, SQLite 3.53.4 CLI SIGSEGV in sqlite3InitCallback):
 * lowering kept declared function-pointer signatures in a translation-unit-
 * wide table keyed by bare name and never removed entries.  SQLite's malloc.c
 * declares `void (*xCallback)(void *, sqlite3_int64, int)` (the
 * sqlite3_memory_alarm parameter); later sqlite3_exec's typedef-declared
 * `sqlite3_callback xCallback` registered nothing, so the stale entry supplied
 * arg types [Ptr, I64, I32, Ptr] for `xCallback(pArg, nCol, azVals, azCols)`
 * and `char **azVals` was passed as a 32-bit value (`movl %r13d, %edx`).
 * Sibling defects on the same path, also pinned here:
 *   - a typedef-declared local fn pointer fell back to a SAME-NAMED
 *     function's / builtin's signature (`llf_t abs` became `int abs(int)`
 *     and was constant-folded to 0);
 *   - a typedef-declared VARIADIC fn pointer must still set AL / pass FP
 *     varargs in xmm (its `...` is only visible through the c_type).
 */
#include <stdio.h>

typedef long long i64;
typedef int (*cb_t)(void *, int, char **, char **);
typedef int (*pf_t)(char *, const char *, ...);
typedef i64 (*llf_t)(i64);

#define V ((char **)0x123456789a0ull)
#define C ((char **)0xfedcba98760ull)

static int seen_ok;
static int cb(void *p, int n, char **v, char **c) {
    seen_ok = p == (void *)&seen_ok && n == 3 && v == V && c == C;
    return 7;
}
static i64 alarm_sum;
static void alarm_cb(void *p, i64 used, int n) { (void)p; alarm_sum += used * 10 + n; }
static i64 triple(i64 x) { return x * 3; }

/* 1. An earlier function with a same-named, differently typed parameter. */
__attribute__((noinline)) int alarm_reg(void (*xCallback)(void *pArg, i64 used, int N),
                                        void *pArg) {
    xCallback(pArg, 1, 2);
    return 0;
}
__attribute__((noinline)) int run(cb_t xCallback, void *pArg, int n, char **v, char **c) {
    return xCallback(pArg, n, v, c);
}

/* 2. Block-scope shadowing inside one function, and restoration after it. */
__attribute__((noinline)) int shadow(void *pArg, char **v, char **c) {
    void (*f)(void *, i64, int) = alarm_cb;
    int r;
    f(pArg, 5, 6);
    {
        cb_t f = cb;
        r = f(pArg, 3, v, c);
    }
    f(pArg, 1LL << 40, 9);
    return r;
}

/* 3. A parameter named like a library builtin shadows it. */
__attribute__((noinline)) i64 call_abs(llf_t abs) { return abs(1LL << 40); }

/* 4. Typedef-declared variadic function pointer. */
__attribute__((noinline)) int fmt(pf_t p, char *buf) {
    return p(buf, "%.2f|%d|%.3f", 2.5, 7, -1.125);
}

int main(void) {
    char buf[64];
    int bad = 0, r, n;
    i64 a;

    alarm_reg(alarm_cb, 0);
    r = run(cb, &seen_ok, 3, V, C);
    printf("run=%d ok=%d\n", r, seen_ok);
    bad |= r != 7 || !seen_ok;

    seen_ok = 0;
    r = shadow(&seen_ok, V, C);
    printf("shadow=%d ok=%d alarm=%lld\n", r, seen_ok, alarm_sum);
    bad |= r != 7 || !seen_ok || alarm_sum != 12 + 56 + (1LL << 40) * 10 + 9;

    a = call_abs(triple);
    printf("abs=%lld\n", a);
    bad |= a != 3 * (1LL << 40);

    n = fmt(sprintf, buf);
    printf("fmt=%d '%s'\n", n, buf);
    bad |= n != 13;

    return bad;
}
