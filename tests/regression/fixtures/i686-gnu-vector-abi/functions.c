typedef unsigned char v16u8 __attribute__((vector_size(16)));
typedef unsigned short v8u16 __attribute__((vector_size(16)));
typedef unsigned long long v2u64 __attribute__((vector_size(16)));
typedef unsigned v4u __attribute__((vector_size(16)));
typedef double v2d __attribute__((vector_size(16)));
v16u8 id8(v16u8 a) { return a; }
v8u16 id16(v8u16 a) { return a; }
v2u64 id64(v2u64 a) { return a; }
v2d iddouble(v2d a) { return a; }
v4u id32(v4u a) { return a; }
v4u relay32(v4u a, v4u (*f)(v4u)) { return f(a); }
v4u pick5(v4u a, v4u b, v4u c, v4u d, v4u e) {
    (void)a; (void)c; (void)d;
    return __builtin_shufflevector(b,e,0,5,2,7);
}
#ifndef LCCC_OMIT_VARIADIC_ABI_PROBE
v4u pick_variadic(v4u a, ...) {
    __builtin_va_list ap;
    __builtin_va_start(ap,a);
    v4u b = __builtin_va_arg(ap,v4u);
    __builtin_va_end(ap);
    return __builtin_shufflevector(a,b,0,5,2,7);
}
#endif
