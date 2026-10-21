/* Affine addresses across calls: scales 1/2/4/8/16, partial trips and guards.
 * No host headers: the same source is also a cross-target emission fixture. */
extern int printf(const char *, ...);
typedef unsigned long long U64;
struct Wide { U64 value, guard; };

__attribute__((noinline)) static U64 step(unsigned i) {
    return (U64)i * 0x9e3779b9ULL + 0x123456789ULL;
}

#define DEFINE_CASE(NAME, TYPE) \
__attribute__((noinline)) static U64 NAME(unsigned n) { \
    TYPE src[65], dst[65]; \
    for (unsigned i = 0; i < 65; ++i) { \
        src[i] = (TYPE)((U64)i * 17 + 3); \
        dst[i] = (TYPE)0xa5; \
    } \
    for (unsigned i = 0; i < n; ++i) \
        dst[i + 1] = (TYPE)(src[i + 1] + step(i)); \
    U64 hash = 0; \
    for (unsigned i = 0; i < 65; ++i) \
        hash = hash * 131 + dst[i]; \
    return hash; \
}
DEFINE_CASE(bytes, unsigned char)
DEFINE_CASE(halves, unsigned short)
DEFINE_CASE(words, unsigned int)
DEFINE_CASE(quads, U64)

__attribute__((noinline)) static U64 wide(unsigned n) {
    struct Wide src[65], dst[65];
    for (unsigned i = 0; i < 65; ++i) {
        src[i].value = (U64)i * 17 + 3;
        src[i].guard = 0;
        dst[i].value = 0xa5;
        dst[i].guard = 0xdeadbeefULL ^ i;
    }
    for (unsigned i = 0; i < n; ++i)
        dst[i + 1].value = src[i + 1].value + step(i);
    U64 hash = 0;
    for (unsigned i = 0; i < 65; ++i) {
        hash = hash * 131 + dst[i].value;
        hash = hash * 131 + dst[i].guard;
    }
    return hash;
}

int main(void) {
    const unsigned counts[] = {0, 1, 2, 3, 7, 15, 31, 63, 64};
    for (unsigned i = 0; i < sizeof counts / sizeof counts[0]; ++i) {
        unsigned n = counts[i];
        printf("%u %llu %llu %llu %llu %llu\n", n,
               bytes(n), halves(n), words(n), quads(n), wide(n));
    }
    return 0;
}
