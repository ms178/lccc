#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <immintrin.h>
extern void f0(void *, const void *, const void *);
extern void f1(void *, const void *, const void *);
extern void f2(void *, const void *, const void *);
extern void f3(void *, const void *, const void *);
extern void f4(void *, const void *, const void *);
extern void f5(void *, const void *, const void *);
extern void f6(void *, const void *, const void *);
extern void f7(void *, const void *, const void *);
extern void f8(void *, const void *, const void *);
extern void f9(void *, const void *, const void *);
extern void f10(void *, const void *, const void *);
extern void f11(void *, const void *, const void *);
extern void f12(void *, const void *, const void *);
extern void f13(void *, const void *, const void *);
extern void f14(void *, const void *, const void *);
extern void f15(void *, const void *, const void *);
extern void f16(void *, const void *, const void *);
extern void f17(void *, const void *, const void *);
extern void f18(void *, const void *, const void *);
extern void f19(void *, const void *, const void *);
extern void f20(void *, const void *, const void *);
extern void f21(void *, const void *, const void *);
extern void f22(void *, const void *, const void *);
extern void f23(void *, const void *, const void *);
extern void f24(void *, const void *, const void *);
extern void f25(void *, const void *, const void *);
extern void f26(void *, const void *, const void *);
extern void f27(void *, const void *, const void *);
extern void f28(void *, const void *, const void *);
extern void f29(void *, const void *, const void *);
extern void f30(void *, const void *, const void *);
extern void f31(void *, const void *, const void *);
extern void f32(void *, const void *, const void *);
extern void f33(void *, const void *, const void *);
extern void f34(void *, const void *, const void *);
static const struct { void (*fn)(void *,const void *,const void *); unsigned width,vex,same; } cases[] = {
{f0,8,0,0},
{f1,8,0,0},
{f2,8,0,1},
{f3,8,0,1},
{f4,8,0,0},
{f5,8,0,0},
{f6,8,0,1},
{f7,8,0,1},
{f8,4,0,0},
{f9,4,0,0},
{f10,4,0,1},
{f11,4,0,1},
{f12,4,0,0},
{f13,4,0,0},
{f14,4,0,1},
{f15,4,0,1},
{f16,8,1,0},
{f17,8,1,0},
{f18,8,1,1},
{f19,8,1,1},
{f20,8,1,0},
{f21,8,1,0},
{f22,8,1,1},
{f23,8,1,1},
{f24,4,1,0},
{f25,4,1,0},
{f26,4,1,1},
{f27,4,1,1},
{f28,4,1,0},
{f29,4,1,0},
{f30,4,1,1},
{f31,4,1,1},
{f32,8,0,0},
{f33,8,0,1},
{f34,8,0,1},
};
int main(void) {
    static const uint64_t bits[] = {0, UINT64_MAX, UINT64_C(0x8000000000000000),
        UINT64_C(0x7ff0000000000001), UINT64_C(0x7ff8000012345678),
        UINT64_C(0x7ff0000000000000), UINT64_C(0x0123456789abcdef), 1, UINT64_C(0x7f800001),
        UINT64_C(0x7fc12345), UINT64_C(0x80000000), UINT64_C(0x7f800000)};
    unsigned failures=0, checks=0;
    unsigned saved=_mm_getcsr();
    for (unsigned mode=0;mode<16;++mode) {
    _mm_setcsr((saved & ~0xe07fu) | ((mode & 3) << 13) |
               ((mode & 4) ? 0x8000u : 0) | ((mode & 8) ? 0x40u : 0) | 0x21u);
    for (unsigned i=0;i<sizeof(cases)/sizeof(cases[0]);++i) {
        for (unsigned k=0;k<sizeof(bits)/sizeof(bits[0]);++k) {
            unsigned char src[32], old[32], got[32], expected[32];
            for (unsigned b=0;b<32;++b) { src[b]=(unsigned char)(13*b+19); old[b]=(unsigned char)(7*b+91); }
            memcpy(src,&bits[k],8);
            memcpy(expected,cases[i].same?src:old,32);
            memset(expected,0,cases[i].vex?32:16);
            memcpy(expected,src,cases[i].width);
            memset(got,0xa5,32);
            unsigned mxcsr=_mm_getcsr();
            cases[i].fn(got,src,old);
            if (memcmp(got,expected,32) || _mm_getcsr()!=mxcsr) {
                printf("FAIL case=%u pattern=%u\n",i,k); ++failures;
            }
            ++checks;
        }
    }
    }
    _mm_setcsr(saved);
    printf("scalar transfers: %u checks, %u failures\n",checks,failures);
    return failures!=0;
}
