/* The kernel's fortified string functions (include/linux/fortify-string.h):
 *
 *     extern __always_inline __gnu_inline
 *     int memcmp(const void *p, const void *q, size_t size)
 *     { ...checks...; return __builtin_memcmp(p, q, size); }
 *
 * The body is an inline-only wrapper AROUND the library function of the same
 * name, and `__builtin_memcmp` inside it is a call to the library.  LCCC
 * lowered the builtin to a call of "memcmp" and then read that as a call of
 * the wrapper: it inlined the wrapper into itself (468 times in an 18-line
 * file) and turned the "self call" into a loop.  Linux's `__jump_label_patch`
 * had no `ret` and no `memcmp`; it ended in a jump to itself.
 *
 * Each wrapper below must reach the library exactly once per call, and the
 * check in front of it must stay.
 */
#include <stddef.h>

extern int printf(const char *, ...);
extern void abort(void);

#define FORTIFY extern inline __attribute__((gnu_inline, always_inline))

static int panics;
__attribute__((noinline)) static void fortify_panic(const char *what)
{
    (void)what;
    panics++;
}

FORTIFY int memcmp(const void *p, const void *q, size_t size)
{
    if (size > 4096)
        fortify_panic("memcmp");
    return __builtin_memcmp(p, q, size);
}
FORTIFY void *memcpy(void *p, const void *q, size_t size)
{
    if (size > 4096)
        fortify_panic("memcpy");
    return __builtin_memcpy(p, q, size);
}
FORTIFY void *memset(void *p, int c, size_t size)
{
    if (size > 4096)
        fortify_panic("memset");
    return __builtin_memset(p, c, size);
}
FORTIFY size_t strlen(const char *p)
{
    size_t n = __builtin_strlen(p);
    if (n > 4096)
        fortify_panic("strlen");
    return n;
}
/* A wrapper that calls another wrapper, as `strcpy` does in the kernel. */
FORTIFY char *strcpy(char *p, const char *q)
{
    size_t n = strlen(q) + 1;
    memcpy(p, q, n);
    return p;
}

struct patch {
    unsigned char code[5];
    unsigned char nop[5];
};

/* The shape of `__jump_label_patch`: compare, complain, copy. */
__attribute__((noinline)) int patch_site(struct patch *site, const unsigned char *expect,
                                         const unsigned char *insn, size_t len)
{
    if (memcmp(site->code, expect, len)) {
        fortify_panic("unexpected op");
        return -1;
    }
    memcpy(site->code, insn, len);
    return 0;
}
__attribute__((noinline)) size_t measure(const char *a, const char *b)
{
    return strlen(a) * 100 + strlen(b);
}
__attribute__((noinline)) int fill_and_compare(unsigned char *a, unsigned char *b, size_t n, int c)
{
    memset(a, c, n);
    memset(b, c, n);
    b[n / 2] ^= 1;
    return memcmp(a, b, n) < 0 ? -1 : memcmp(a, b, n) > 0;
}
/* The size reaches the wrapper as a variable: the check cannot fold. */
__attribute__((noinline)) int checked(size_t n)
{
    static unsigned char big[8192], other[8192];
    int before = panics;
    memcpy(big, other, n);
    return panics - before;
}

int main(void)
{
    struct patch site = { { 1, 2, 3, 4, 5 }, { 0x0f, 0x1f, 0x44, 0, 0 } };
    const unsigned char expect[5] = { 1, 2, 3, 4, 5 };
    const unsigned char wrong[5] = { 1, 2, 3, 4, 6 };
    unsigned char a[64], b[64];
    char text[32];

    /* One call per statement: the order of evaluation of arguments is not
     * specified, and the two compilers differ. */
    int first = patch_site(&site, expect, site.nop, 5);
    printf("patch %d\n", first);
    printf("code %d %d %d\n", site.code[0], site.code[1], site.code[2]);
    int second = patch_site(&site, wrong, site.nop, 5);
    printf("patch again %d panics %d\n", second, panics);
    printf("measure %d\n", (int)measure("jump", "label patch"));
    printf("fill %d\n", fill_and_compare(a, b, sizeof(a), 0x5a));
    strcpy(text, "fortified");
    printf("strcpy %s %d\n", text, (int)strlen(text));
    int small = checked(100);
    int large = checked(5000);
    printf("checked %d %d\n", small, large);
    if (panics != 2)
        abort();
    return 0;
}
