/* A translation unit that defines both string functions its own code then
 * uses, in the exact shapes the optimizer rewrites:
 *
 *   - `forward_copy`'s loop is a disjoint forward copy (the `loop_idiom`
 *     target).  Before A13 the rewrite emitted `call memmove@PLT`; because
 *     this TU defines `memcpy`, the library `memmove` (newlib, historical
 *     glibc/BSD) can reach this TU's deliberately-reverse `memcpy`, so the
 *     "optimization" changed the program's result or recursed.
 *   - `memset(b, 7, 8)` is a constant-size call to a TU-defined `memset`
 *     that ignores `c`.  Before A14b the backend baked the library fill
 *     (`0x07...07`), silently ignoring this definition.
 *
 * Post-fix both shapes do what the C program says: the loop stays scalar and
 * the memset call reaches this TU's function.  GCC agrees (exit 0).
 */
typedef __SIZE_TYPE__ size_t;

void *memcpy(void *d, const void *s, size_t n) {
    unsigned char *p = d;
    const unsigned char *q = s;
    while (n--) p[n] = q[n]; /* deliberately reverse: observable */
    return d;
}

void *memset(void *d, int c, size_t n) {
    unsigned char *p = d;
    for (size_t i = 0; i < n; i++) p[i] = 0; /* ignores c */
    return d;
}

void forward_copy(unsigned char *restrict d, const unsigned char *restrict s,
                  unsigned n) {
    for (unsigned i = 0; i < n; i++) d[i] = s[i];
}

int main(void) {
    unsigned char b[8];
    const unsigned char s[4] = {'a', 'b', 'c', 'd'};

    forward_copy(b, s, 4);
    /* A rewritten loop would have run the TU's reverse memcpy instead. */
    if (b[0] != 'a' || b[3] != 'd') return 1;

    memset(b, 7, 8);
    /* A library-contract expansion would have written seven-bytes. */
    if (b[0] != 0 || b[7] != 0) return 2;

    return 0;
}
