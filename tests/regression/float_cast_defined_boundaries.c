/* Defined C conversions: truncation and representable binary32 boundaries. */
#include <limits.h>
__attribute__((noinline)) static int convert(float x) { return (int)x; }
int main(void) {
    if (convert(0.0f) != 0 || convert(-0.0f) != 0) return 1;
    if (convert(3.75f) != 3 || convert(-3.75f) != -3) return 2;
#if INT_MAX == 2147483647
    if (convert(2147483520.0f) != 2147483520) return 3;
    if (convert(-2147483648.0f) != INT_MIN) return 4;
#endif
    return 0;
}
