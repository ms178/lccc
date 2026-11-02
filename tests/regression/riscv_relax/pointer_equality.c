#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdio.h>
#include <string.h>

/* An imported function's address taken in position-dependent code is its
   PLT entry, and that must equal the address every shared object sees.
   GNU ld exports the PLT entry as the undefined symbol's st_value, so the
   dynamic linker makes it canonical (riscv_elf_finish_dynamic_symbol).
   Without it, `mine` holds the PLT stub while libc resolves strlen to its
   own copy, and function-pointer comparisons across objects fail. */
size_t (*volatile mine)(const char *) = strlen;

int main(void)
{
    void *theirs = dlsym(RTLD_DEFAULT, "strlen");
    printf("%s\n", (void *)mine == theirs ? "equal" : "DIFFERENT");
    return (void *)mine != theirs;
}
