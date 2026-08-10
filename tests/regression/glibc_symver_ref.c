/* glibc_symver_ref.c — versioned .symver REFERENCES (glibc
 * compat_symbol_reference). A TU that references `printf`/`memcpy` via
 * `.symver real, name@VER` must emit versioned references and the linker must
 * resolve them against libc.so.6's exported versions (default-version ref for
 * printf@GLIBC_2.2.5, and a NON-default version ref for memcpy@GLIBC_2.2.5 —
 * memcpy's default is GLIBC_2.14). Regression: LCCC's linker treated
 * "name@VER" as an opaque undefined name and reported it unresolved, blocking
 * glibc builds. The original test used `_LIB_VERSION`/`matherr`, which glibc
 * 2.36+ no longer exports — unlinkable with ANY compiler on modern hosts. */
#include <stdio.h>
#include <string.h>

extern int printf(const char *, ...);
extern void *memcpy(void *, const void *, unsigned long);

__asm__(".symver printf,printf@GLIBC_2.2.5");
__asm__(".symver memcpy,memcpy@GLIBC_2.2.5");

int main(void) {
    /* Taking addresses forces versioned references into the object. */
    volatile unsigned long a = (unsigned long)&printf;
    volatile unsigned long b = (unsigned long)&memcpy;
    if (a == 0 || b == 0) { printf("FAIL symver refs\n"); return 1; }
    /* Call through the versioned refs: default version (printf) and
     * non-default version (memcpy@GLIBC_2.2.5). */
    char dst[16];
    memcpy(dst, "symver-ok", 10);
    if (strncmp(dst, "symver-ok", 9) != 0) { printf("FAIL memcpy via symver\n"); return 1; }
    printf("PASS symver_ref\n");
    return 0;
}
