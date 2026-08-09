/* glibc_symver_ref.c — versioned .symver REFERENCES (glibc
 * compat_symbol_reference). A TU that only REFERENCES `matherr`/`_LIB_VERSION`
 * via `.symver real, name@VER` must emit versioned references; GNU ld 2.47
 * refuses to bind unversioned refs to foo@VER under --whole-archive (the
 * libm.so link failure). We verify the object carries the versioned refs by
 * resolving through the version script at runtime. */
#include <stdio.h>

extern int _LIB_VERSION;
extern int matherr(void *);

__asm__(".symver _LIB_VERSION,_LIB_VERSION@GLIBC_2.2.5");
__asm__(".symver matherr,matherr@GLIBC_2.2.5");

int main(void) {
    /* Taking addresses forces versioned references into the object. */
    volatile unsigned long a = (unsigned long)&_LIB_VERSION;
    volatile unsigned long b = (unsigned long)&matherr;
    if (a == 0 || b == 0) { printf("FAIL symver refs\n"); return 1; }
    printf("PASS symver_ref %lx %lx\n", a, b);
    return 0;
}
