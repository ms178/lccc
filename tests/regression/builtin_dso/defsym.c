/* --defsym=getpagesize=my_getpagesize: the reference must bind to this
   object's definition, not to libc's same-named import. */
#include <stdio.h>
#include <unistd.h>
int my_getpagesize(void) { return 12345; }
int main(void) {
    printf("%d\n", getpagesize());
    return 0;
}
