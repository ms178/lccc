/* Deliberately separate translation unit: the copy TU sees two *extern*
 * names, not the alias attribute from defs.c. Distinct names are not proof
 * of disjoint storage at link time. */
extern unsigned char X[64], Y[64];
__attribute__((noinline)) void copy_extern_alias(unsigned n) {
    unsigned char *dst = Y + 1;
    for (unsigned i = 0; i < n; ++i) *dst++ = X[i];
}
