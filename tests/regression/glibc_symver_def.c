/* glibc_symver_def.c — default (@@) .symver definitions (glibc
 * default_symbol_version: `__libc_start_main@@GLIBC_2.34`). LCCC's old
 * handler dropped the @@ alias when base == name -> no default version in
 * the object -> ld could not bind unversioned references. Verified against
 * binutils 2.47: `.symver real, name@@V` emits BOTH `real` and `name@@V`
 * (here: internal_fn + public_fn@@GLIBC_2.34). */
#include <stdio.h>

__attribute__((noinline)) int internal_fn(int x) { return x + 1; }
__asm__(".symver internal_fn, public_fn@@GLIBC_2.34");

int main(void) {
    if (internal_fn(41) != 42) { printf("FAIL symver def\n"); return 1; }
    printf("PASS symver_def\n");
    return 0;
}
