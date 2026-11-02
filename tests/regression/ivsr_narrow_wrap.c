/* IVSR-WRAP-1: promoted Add followed by truncation is not a pointer IV.
 * All subscripts are in bounds before optimization. The independent
 * reference computes the modulo index in unsigned int, without a narrow IV.
 */
#include <stdio.h>
static unsigned int bytes[256], words[65536];
__attribute__((noinline)) static unsigned int scan8(unsigned char x, int n) {
    unsigned int sum = 0;
    for (int k = 0; k < n; ++k, ++x) sum += bytes[x];
    return sum;
}
__attribute__((noinline)) static unsigned int scan16(unsigned short x, int n) {
    unsigned int sum = 0;
    for (int k = 0; k < n; ++k, ++x) sum += words[x];
    return sum;
}
int main(void) {
    for (unsigned int i = 0; i < 256; ++i) bytes[i] = i * 7 + 3;
    for (unsigned int i = 0; i < 65536; ++i) words[i] = i * 11 + 5;
    for (unsigned int start = 240; start < 256; ++start) {
        for (int n = 0; n <= 520; n += 13) {
            unsigned int a = 0, b = 0;
            for (int k = 0; k < n; ++k) {
                a += bytes[(start + (unsigned int)k) & 255];
                b += words[(65280 + start + (unsigned int)k) & 65535];
            }
            if (scan8((unsigned char)start, n) != a ||
                scan16((unsigned short)(65280 + start), n) != b) return 1;
        }
    }
    puts("ivsr_narrow_wrap: OK");
    return 0;
}
