/* LLVM https://github.com/llvm/llvm-project/issues/233753
 * C adaptation uses unsigned modular negation: no abs(INT_MIN) UB.
 * Both equality polarities, arm orders, high bits and observable conditions. */
#include <stdint.h>
#include <stdio.h>

#define NOINLINE __attribute__((noinline))
NOINLINE int zero_eq32(uint32_t x, unsigned c) { return (c ? -x : x) == 0; }
NOINLINE int zero_ne32(uint32_t x, unsigned c) { return (c ? x : -x) != 0; }
NOINLINE int zero_eq64(uint64_t x, unsigned c) { return 0 == (c ? x : -x); }
NOINLINE int zero_ne64(uint64_t x, unsigned c) { return 0 != (c ? -x : x); }
NOINLINE int observed_condition(uint64_t x, volatile unsigned *c) {
    return (*c ? -x : x) == 0;
}
NOINLINE int different_root(uint64_t x, uint64_t y, unsigned c) {
    return (c ? -x : y) == 0;
}
NOINLINE int not_negation(uint64_t x, unsigned c) { return (c ? ~x : x) == 0; }
NOINLINE int narrowing(uint64_t x, unsigned c) {
    return (uint32_t)(c ? -x : x) == 0;
}

int main(void) {
    static const uint64_t edges[] = {0,1,2,UINT32_MAX,UINT64_C(0x80000000),
        UINT64_C(0x100000000),UINT64_C(0x8000000000000000),UINT64_MAX};
    uint64_t x = 0, hash = 0;
    for (unsigned i=0;i<65544;++i) {
        x = i<8 ? edges[i] : x*UINT64_C(6364136223846793005)+1;
        for (unsigned c=0;c<4;++c) {
            if (zero_eq32((uint32_t)x,c) != ((uint32_t)x==0) ||
                zero_ne32((uint32_t)x,c) != ((uint32_t)x!=0) ||
                zero_eq64(x,c) != (x==0) || zero_ne64(x,c) != (x!=0)) return 1;
            volatile unsigned condition = c;
            if (observed_condition(x,&condition) != (x==0)) return 2;
            volatile uint64_t alternative = c ? -x : (x^UINT64_MAX);
            if (different_root(x,x^UINT64_MAX,c) != (alternative==0)) return 3;
            alternative = c ? ~x : x;
            if (not_negation(x,c) != (alternative==0)) return 4;
            alternative = c ? -x : x;
            if (narrowing(x,c) != ((uint32_t)alternative==0)) return 5;
            hash += (uint32_t)x + (uint64_t)c;
        }
    }
    printf("%016llx\n",(unsigned long long)hash);
    return 0;
}
