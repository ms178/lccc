/* A shared object calling imports and its own preemptible `hook`.
   Relaxation turns `call printf` into `jal` to printf's PLT entry; lccc-ld
   once dropped the PLT after relaxing, leaving `jal 0` (or a c.j overflow
   error) in every such library. `hook` is a default-visibility definition,
   so the program's `hook` preempts it (ELF interposition) unless the
   library is linked -Bsymbolic or -Bsymbolic-functions.
   SHARED_IMPORT_MAIN builds the bfd-linked program that loads it. */
#include <stdio.h>
#include <string.h>

int hook(void);
#ifdef SHARED_IMPORT_MAIN
int greet(const char *s);
int hook(void)
{
    return 2;
}
int main(void)
{
    return greet("world") != 16; /* "hello world 5 N\n" */
}
#else
int hook(void)
{
    return 1;
}
int greet(const char *s)
{
    return printf("hello %s %zu %d\n", s, strlen(s), hook());
}
#endif
