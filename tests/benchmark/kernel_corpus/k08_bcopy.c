/* Fixed-size aggregate copy. */
struct S64 { char b[64]; };

void copy64(void *d, const void *s) {
    *(struct S64 *)d = *(const struct S64 *)s;
}
