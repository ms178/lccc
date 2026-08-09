#include <stdint.h>
#include <string.h>
static __attribute__((noinline)) double dcalc(double a, double b, double c) {
    double x = a + b;
    double y = x * c;
    return (y - b) / (a + 1.25);
}
static __attribute__((noinline)) float fcalc(float a, float b, float c) {
    float x = a + b;
    float y = x * c;
    return (y - b) / (a + 1.25f);
}
int main(void) {
    uint64_t h = 0;
    for (int i = 1; i < 200; ++i) {
        double d = dcalc(i * .03125 + 1.0, i * .015625 + .5, .25);
        float f = fcalc(i * .03125f + 1.0f, i * .015625f + .5f, .25f);
        uint64_t u; uint32_t v;
        memcpy(&u, &d, 8); memcpy(&v, &f, 4);
        h ^= u + (uint64_t)v * UINT64_C(0x9e3779b1);
    }
    return h != UINT64_C(0x350f46f06cf290aa);
}
