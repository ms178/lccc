// C11 6.3.1.8 usual arithmetic conversions: same-width signed/unsigned pairs.
// Historical bug: when the signed type had higher rank but the SAME width
// (long long vs unsigned long on LP64), the else-arm returned the signed
// type; the standard requires the unsigned type corresponding to the signed
// type. Frontend constant evaluation (enum init) and _Generic both exercise
// the sema-side conversion; the volatile globals exercise the runtime path.
#include <stdio.h>

enum { E_ULL_WINS = (-1LL + 0UL) > 0 };   /* ULL arithmetic: huge > 0 -> 1 */

volatile long long g_ll = -1;
volatile unsigned long g_ul = 0;
volatile long g_l = -1;
volatile unsigned int g_ui = 0;

int main(void) {
    /* LP64: LL + UL (both 64-bit) -> unsigned long long */
    printf("%d\n", E_ULL_WINS);
    printf("%s\n", _Generic(g_ll + g_ul,
                            unsigned long long: "ull",
                            long long: "ll",
                            default: "other"));
    /* runtime path: (huge unsigned) < 0 must be false */
    printf("%d\n", (g_ll + g_ul) < 0);
    /* LP64: L + UI -> long CAN represent all unsigned int -> signed long */
    printf("%s\n", _Generic(g_l + g_ui,
                            long: "l",
                            unsigned long: "ul",
                            default: "other"));
    printf("%d\n", (g_l + g_ui) < 0);
    /* division through the converted (unsigned) type */
    volatile long long d_ll = -2;
    volatile unsigned long d_ul = 0;
    printf("%llu\n", (unsigned long long)((d_ll + d_ul) / 2));
    return 0;
}
