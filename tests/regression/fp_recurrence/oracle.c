/* Minimal strict-FP doubling/contraction oracle; all inputs are finite and
 * exactly representable. The checksum covers both widths and all consumers. */
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#define NI __attribute__((noinline))
NI double twice_plus(double x, double y) { return 2.0*x+y; }
NI double twice_minus(double x, double y) { return x*2.0-y; }
NI float twice_plus32(float x, float y) { return x*2.0f+y; }
NI double orbit(double c, int n) {
    double x = 0.0, total = 0.0;
    for (int i = 0; i < n; ++i) {
        double old = x*x;
        x = old + c;
        total += old + x*x;
    }
    return total + x;
}
int main(void) {
    uint64_t hash = 14695981039346656037ULL;
    for (int i = -128; i <= 128; ++i) {
        double x = i / 256.0, y = (i + 19) / 512.0;
        double a = twice_plus(x, y), b = twice_minus(x, y);
        double d = orbit(x / 4.0, 8);
        float c = twice_plus32((float)x, (float)y);
        uint64_t u, v, w; uint32_t z;
        memcpy(&u, &a, 8); memcpy(&v, &b, 8);
        memcpy(&w, &d, 8); memcpy(&z, &c, 4);
        hash = (hash ^ u) * 1099511628211ULL;
        hash = (hash ^ v) * 1099511628211ULL;
        hash = (hash ^ z) * 1099511628211ULL;
        hash = (hash ^ w) * 1099511628211ULL;
    }
    printf("%016llx\n", (unsigned long long)hash);
    return 0;
}
