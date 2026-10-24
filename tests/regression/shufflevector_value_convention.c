/* Typed constant shuffles: aggregate lifetime, ABI, and composition witnesses. */
#include <stdint.h>
#include <stdio.h>
#include <string.h>
typedef uint32_t v4u __attribute__((vector_size(16)));
typedef float v4f __attribute__((vector_size(16)));

v4u shuffle_reverse_twice(v4u);
v4u shuffle_nested(v4u, v4u);
v4u shuffle_mixed(v4u, v4u);
v4u shuffle_second(v4u, v4u);
v4u shuffle_field(v4u, v4u, int);
v4u shuffle_argument(v4u, v4u);
v4f shuffle_float(v4f);
unsigned shuffle_effects(v4u, v4u);

#ifndef SHUFFLE_DRIVER_ONLY
__attribute__((noinline)) v4u shuffle_reverse_twice(v4u a) {
    v4u b = __builtin_shufflevector(a, a, 3, 2, 1, 0);
    return __builtin_shufflevector(b, b, 3, 2, 1, 0);
}
__attribute__((noinline)) v4u shuffle_nested(v4u a, v4u b) {
    return __builtin_shufflevector(__builtin_shufflevector(a,b,0,5,2,7),a,3,2,1,0);
}
__attribute__((noinline)) v4u shuffle_mixed(v4u a, v4u b) {
    return __builtin_shufflevector(a,b,0,5,2,7);
}
__attribute__((noinline)) v4u shuffle_second(v4u a, v4u b) {
    return __builtin_shufflevector(a,b,7,6,5,4);
}
__attribute__((noinline)) v4u shuffle_field(v4u a, v4u b, int condition) {
    struct holder { v4u value; } h;
    h.value = condition ? __builtin_shufflevector(a,b,3,2,1,0)
                        : __builtin_shufflevector(a,b,4,5,6,7);
    return h.value;
}
__attribute__((noinline)) static v4u consume(v4u x) { return x; }
__attribute__((noinline)) v4u shuffle_argument(v4u a, v4u b) {
    return consume(__builtin_shufflevector(a,b,0,5,2,7));
}
__attribute__((noinline)) v4f shuffle_float(v4f a) {
    return __builtin_shufflevector(a,a,3,2,1,0);
}
static unsigned evaluations;
__attribute__((noinline)) static v4u evaluated(v4u a) { ++evaluations; return a; }
__attribute__((noinline)) unsigned shuffle_effects(v4u a, v4u b) {
    evaluations = 0;
    v4u x = __builtin_shufflevector(evaluated(a), evaluated(b),0,1,2,3);
    _Static_assert(sizeof(__builtin_shufflevector(a,b,0,5,2,7)) == 16, "shuffle size");
    _Static_assert(_Generic(__builtin_shufflevector(a,b,0,5,2,7),v4u:1,default:0), "shuffle type");
    unsigned count = evaluations;
    return count == 2 && x[0] == a[0] && x[3] == a[3];
}
#endif

#ifndef SHUFFLE_FUNCTIONS_ONLY
static int same(v4u a, v4u b) {
    for (unsigned i=0;i<4;i++) if (a[i]!=b[i]) return 0;
    return 1;
}
int main(void) {
    uint32_t state = UINT32_C(0x12345678);
    for (unsigned test=0;test<2048;test++) {
        v4u a, b;
        for (unsigned i=0;i<4;i++) {
            state ^= state<<13; state ^= state>>17; state ^= state<<5; a[i]=state;
            state ^= state<<13; state ^= state>>17; state ^= state<<5; b[i]=state;
        }
        v4u mixed = {a[0],b[1],a[2],b[3]};
        v4u nested = {b[3],a[2],b[1],a[0]};
        v4u reverse = {a[3],a[2],a[1],a[0]};
        v4u second = {b[3],b[2],b[1],b[0]};
        if (!same(shuffle_reverse_twice(a),a) || !same(shuffle_nested(a,b),nested)
            || !same(shuffle_mixed(a,b),mixed) || !same(shuffle_second(a,b),second)
            || !same(shuffle_field(a,b,1),reverse) || !same(shuffle_field(a,b,0),b)
            || !same(shuffle_argument(a,b),mixed) || !shuffle_effects(a,b)) return 1;
        /* Preserve every bit, including NaN payloads, without FP arithmetic. */
        v4f floats; memcpy(&floats,&a,16);
        floats=shuffle_float(floats);
        v4u bits; memcpy(&bits,&floats,16);
        if (!same(bits,reverse)) return 2;
    }
    puts("18432 typed shuffle checks");
    return 0;
}
#endif
