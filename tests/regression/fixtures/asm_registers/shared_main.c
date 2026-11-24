/* The program that links against shared_library.c: it writes `counter`
   itself, so the library's statement and this file have to mean the same
   object. */
#include <stdio.h>

extern int counter;
extern __thread int library_tls;
int library_bump(void);
int library_tls_bump(void);
int library_read(void);

int main(void) {
    counter = 100;
    int a = library_bump();
    printf("statement %d program %d library %d\n", a, counter, library_read());
    library_tls = 7;
    printf("tls %d %d\n", library_tls_bump(), library_tls);
    return 0;
}
