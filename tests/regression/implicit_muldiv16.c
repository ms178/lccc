/* MULW/DIVW/IMULW/IDIVW preserve the upper bits of RAX and RDX.
 * The peephole effect matrix separately prevents full-register kill proofs. */
#include <stdint.h>
#if defined(__x86_64__)
int main(void) {
    uint64_t a = UINT64_C(0x123456789abc0007);
    uint64_t d = UINT64_C(0xfedcba9876542468);
    uint16_t k = 9;
    __asm__ volatile("mulw %w2" : "+a"(a), "+d"(d) : "r"(k) : "cc");
    if (a != UINT64_C(0x123456789abc003f) || d != UINT64_C(0xfedcba9876540000)) return 1;
    __asm__ volatile("divw %w2" : "+a"(a), "+d"(d) : "r"(k) : "cc");
    if (a != UINT64_C(0x123456789abc0007) || d != UINT64_C(0xfedcba9876540000)) return 2;
    a = UINT64_C(0x123456789abc8ad0);
    d = UINT64_C(0xfedcba9876542468);
    k = 3;
    __asm__ volatile("imulw %w2" : "+a"(a), "+d"(d) : "r"(k) : "cc");
    if (a != UINT64_C(0x123456789abca070) || d != UINT64_C(0xfedcba987654fffe)) return 3;
    __asm__ volatile("idivw %w2" : "+a"(a), "+d"(d) : "r"(k) : "cc");
    return a != UINT64_C(0x123456789abc8ad0) || d != UINT64_C(0xfedcba9876540000);
}
#else
/* This architectural witness has no meaning on another ISA. */
int main(void) { return 0; }
#endif
