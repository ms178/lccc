/* Packed vector call results must become objects before subscripting.
 * Build with -DVECTOR_PROVIDER for the separate provider translation unit.
 * No vector-to-scalar casts or out-of-range/overflowing arithmetic. */
#define SHAPES(X) \
    X(s8, signed char, 1, -97) \
    X(u8, unsigned char, 1, 251) \
    X(s16, short, 1, -30001) \
    X(u16, unsigned short, 1, 60001) \
    X(s32, int, 1, -1234567) \
    X(u32, unsigned int, 1, 4000000001U) \
    X(s64, long long, 1, -4294967297LL) \
    X(u64, unsigned long long, 1, 0xdeadbeefcafebabeULL) \
    X(f32, float, 1, 1.75f) \
    X(f64, double, 1, -2.25) \
    X(bytes3, unsigned char, 3, 193) \
    X(words2, unsigned int, 2, 1234567) \
    X(floats3, float, 3, 1.25f) \
    X(doubles2, double, 2, 2.5) \
    X(x87, long double, 1, 123.5L)
#define TYPE(name, elem, lanes, value) \
    typedef elem name __attribute__((ext_vector_type(lanes))); \
    extern name make_##name(void);
SHAPES(TYPE)
#ifndef VECTOR_CALLER
#define PROVIDER(name, elem, lanes, value) \
    __attribute__((noinline)) name make_##name(void) { \
        name v = {0}; \
        for (int i = 0; i < lanes; ++i) v[i] = value; \
        return v; \
    }
SHAPES(PROVIDER)
#endif
#ifndef VECTOR_PROVIDER
static int calls;
static int index_value;
static int index_once(void) { ++calls; return index_value; }
#define CHECK(name, elem, lanes, value) do { \
    ++shape; \
    name (*volatile fp)(void) = make_##name; \
    for (int i = 0; i < lanes; ++i) { \
        index_value = i; \
        calls = 0; \
        if (make_##name()[index_once()] != value || calls != 1) return 4 * (shape - 1) + 1; \
        calls = 0; \
        if (fp()[index_once()] != value || calls != 1) return 4 * (shape - 1) + 2; \
        calls = 0; \
        if ((++calls, make_##name())[i] != value || calls != 1) return 4 * (shape - 1) + 3; \
        if (({ make_##name(); })[i] != value) return 4 * (shape - 1) + 4; \
    } \
} while (0);
int main(void) {
    int shape = 0;
    SHAPES(CHECK)
    return 0;
}
#endif
