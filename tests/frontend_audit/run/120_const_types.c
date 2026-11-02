#include <stdio.h>
#define TYPE(x) _Generic((x), int: 1, unsigned: 2, long: 3, unsigned long: 4, \
                         long long: 5, unsigned long long: 6, default: 0)
#define SHOW(x) printf("%-24s %d %lld\n", #x, TYPE(x), (long long)(x))
int main(void) {
    SHOW(1); SHOW(1u); SHOW(1l); SHOW(1ul); SHOW(1ll); SHOW(1ull);
    SHOW(0x7fffffff); SHOW(0x80000000); SHOW(0xffffffff);
    SHOW(0x100000000); SHOW(010); SHOW(0x10);
    SHOW(2147483647); SHOW(2147483648); SHOW(4294967295); SHOW(4294967296);
    SHOW(9223372036854775807); SHOW(0xffffffffffffffff);
    SHOW('a'); SHOW(1.0f); SHOW(1.0); SHOW(1.0l);
    SHOW(-1); SHOW(-2147483648); SHOW(-0x80000000);
    SHOW(1 + 1); SHOW(1u + 1u); SHOW(1u + 1); SHOW(1l + 1u); SHOW(1ll + 1u);
    SHOW(sizeof(int)); SHOW((int)1); SHOW((unsigned)1);
    SHOW(~0); SHOW(~0u); SHOW(!0); SHOW(!1ull);
    SHOW(1 << 2); SHOW(1u << 2); SHOW(1ll << 2);
    return 0;
}
