/* A translation unit that implements the mem* family itself (the shape of a
 * libc or kernel string.c).  The zero path of `memset` is exactly the loop
 * `loop_memset` turns into `memset(p, 0, n)`, and the copy loops of
 * `memmove` are `loop_idiom` shapes: synthesizing those calls here makes
 * `memset` call itself forever (lccc rc=139 at -O2/-O3 before the builtin
 * policy).  A definition of any family member withdraws the whole family
 * from synthesis, so every loop stays a loop.
 *
 * Oracle: GCC 16.2 -O2/-O3 makes the same self-call and hangs (rc=124);
 * Clang 23.1 refuses by name and exits 0.  Hence LCCC_NO_COMPARE (.env).
 */
typedef __SIZE_TYPE__ size_t;

void *memset(void *d, int c, size_t n) {
    unsigned char *p = d;
    if (c == 0) {
        for (size_t i = 0; i < n; i++) p[i] = 0;
        return d;
    }
    for (size_t i = 0; i < n; i++) p[i] = (unsigned char)c;
    return d;
}

void *memmove(void *d, const void *s, size_t n) {
    unsigned char *p = d;
    const unsigned char *q = s;
    if (p < q)
        for (size_t i = 0; i < n; i++) p[i] = q[i];
    else
        while (n--) p[n] = q[n];
    return d;
}

int main(void) {
    volatile size_t n = 100;
    char b[128], c[128];
    memset(b, 0, n);
    memset(c, 5, n);
    memmove(b, c, n);
    return b[99] == 5 && b[0] == 5 ? 0 : 1;
}
