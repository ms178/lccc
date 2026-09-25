/* Separate translation unit: observes hidden only through the public alias. */
#include <stdio.h>
extern int published;
extern void update(int);
int main(void) {
    update(123);
    printf("%d\n", published);
    return published == 123 ? 0 : 1;
}
