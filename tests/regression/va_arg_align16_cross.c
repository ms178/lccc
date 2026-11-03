/* `va_arg` of 16-byte-aligned types after any number of `int`s.
 *
 * AArch64 rounds the general-register save-area offset up to 16 with
 * `add w2, w2, #15; and w2, w2, #-16`.  GNU as and LLVM read `#-16` in a
 * 32-bit logical instruction as its low word 0xfffffff0; LCCC's assembler
 * refused any immediate wider than 32 bits, so every such `va_arg` failed to
 * assemble (gcc.c-torture execute/pr92904.c).
 *
 * Self-checking: exit status 0 is success; abort() otherwise. */
#include <stdarg.h>

extern void abort(void);

struct __attribute__((aligned(16))) A16 { long long a, b; };
struct plain { long long a, b; };

__attribute__((noinline)) static __int128 get_i128(int skip, ...)
{
    va_list ap;
    va_start(ap, skip);
    while (skip--)
        (void)va_arg(ap, int);
    __int128 r = va_arg(ap, __int128);
    va_end(ap);
    return r;
}

__attribute__((noinline)) static struct A16 get_a16(int skip, ...)
{
    va_list ap;
    va_start(ap, skip);
    while (skip--)
        (void)va_arg(ap, int);
    struct A16 r = va_arg(ap, struct A16);
    va_end(ap);
    return r;
}

__attribute__((noinline)) static struct plain get_plain(int skip, ...)
{
    va_list ap;
    va_start(ap, skip);
    while (skip--)
        (void)va_arg(ap, int);
    struct plain r = va_arg(ap, struct plain);
    va_end(ap);
    return r;
}

#define I128(hi, lo) (((__int128)(hi) << 64) | (unsigned long long)(lo))

int main(void)
{
    const __int128 w = I128(0x0123456789abcdefll, 0xfedcba9876543210ull);
    const struct A16 a = {-3, 0x5555aaaa5555aaaall};
    const struct plain p = {11, -12};
    /* Zero to nine leading ints walk the value from registers to the stack,
     * at both parities of the 8-byte slot index. */
    if (get_i128(0, w) != w || get_i128(1, 1, w) != w || get_i128(2, 1, 2, w) != w ||
        get_i128(3, 1, 2, 3, w) != w || get_i128(6, 1, 2, 3, 4, 5, 6, w) != w ||
        get_i128(7, 1, 2, 3, 4, 5, 6, 7, w) != w ||
        get_i128(9, 1, 2, 3, 4, 5, 6, 7, 8, 9, w) != w)
        abort();
    struct A16 r;
    r = get_a16(0, a);
    if (r.a != a.a || r.b != a.b)
        abort();
    r = get_a16(1, 1, a);
    if (r.a != a.a || r.b != a.b)
        abort();
    r = get_a16(7, 1, 2, 3, 4, 5, 6, 7, a);
    if (r.a != a.a || r.b != a.b)
        abort();
    r = get_a16(8, 1, 2, 3, 4, 5, 6, 7, 8, a);
    if (r.a != a.a || r.b != a.b)
        abort();
    struct plain q = get_plain(3, 1, 2, 3, p);
    if (q.a != 11 || q.b != -12)
        abort();
    q = get_plain(8, 1, 2, 3, 4, 5, 6, 7, 8, p);
    if (q.a != 11 || q.b != -12)
        abort();
    return 0;
}
