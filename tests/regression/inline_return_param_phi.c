/* Regression: inliner return-value phi must receive ParamRef substitution.
 * A callee returning its own parameter through a multi-return CFG (switch
 * with break) left a stale value id in the merge phi after inlining:
 * checkCharRefNumber-shape functions returned 0 instead of the parameter.
 * (expat 2.7.3: every XML char ref decoded to 0.) */
#include <stdio.h>
static unsigned char type_tab[256];
static int check(int result) {
    switch (result >> 8) {
    case 0xD8: case 0xD9: case 0xDA: case 0xDB:
    case 0xDC: case 0xDD: case 0xDE: case 0xDF:
        return -1;
    case 0:
        if (type_tab[result] == 0) return -1;
        break;
    case 0xFF:
        if (result == 0xFFFE || result == 0xFFFF) return -1;
        break;
    }
    return result;
}
static int __attribute__((noinline)) crn_a(const char *ptr) {
    int result = 0;
    ptr += 2;
    for (; *ptr != ';'; ptr += 1) {
        result *= 10; result += ((unsigned char)*ptr - '0');
        if (result >= 0x110000) return -1;
    }
    return check(result);
}
static int __attribute__((noinline)) crn_b(const char *ptr) {
    int result = 0;
    ptr += 4;
    for (; *ptr != ';'; ptr += 2) {
        result *= 10; result += ((unsigned char)*ptr - '0');
        if (result >= 0x110000) return -1;
    }
    return check(result);
}
int main(void){
    type_tab[60] = 1; type_tab[34] = 2; type_tab[62] = 3;
    int a = crn_a("&#60;");
    int b = crn_a("&#34;");
    int c = check(0x300);
    printf("%d %d %d %d\n", a, b, c, crn_b("&_#_6_2_;"));
    return (a == 60 && b == 34 && c == 0x300) ? 0 : 1;
}
