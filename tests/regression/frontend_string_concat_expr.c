/* Adjacent string literal concatenation in expressions. */
#include <stdio.h>
int main(void) {
    const char *s = "hel" "lo" "!" ;
    if (s[0] != 'h' || s[4] != 'o' || s[5] != '!') return 1;
    const char *t = "a" "b";
    if (t[0] != 'a' || t[1] != 'b') return 2;
    return 0;
}
