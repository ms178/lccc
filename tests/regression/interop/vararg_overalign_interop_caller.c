#include <stdio.h>
struct __attribute__((aligned(32))) V32 { double a, b, c, d; };
struct __attribute__((aligned(64))) V64 { double a, b, c, d, e, f, g, h; };
struct __attribute__((aligned(32))) N32 { long long a, b; };
extern void take32(int, ...);
extern void take64(int, ...);
extern double named32(int, struct N32, int);
extern void takeM(int, ...);
struct __attribute__((aligned(32))) M32 { long long a, b, c, d; };
struct M32 s32m = { 7, 8, 9, 10 };
struct V32 s32 = { 1.5, 2.5, 3.5, 4.5 };
struct V64 s64 = { 2.5, 3.5, 4.5, 5.5, 6.5, 7.5, 8.5, 9.5 };
/* Exactly-n varargs per call site: the callee trusts n, so the struct must
 * sit at vararg position n. The sweep covers anchor parity 0..12 stack
 * doubles (8 in XMM + 0..4 stack doubles ahead of the struct). */
#define C32(n) take32(n FOR_EACH_12(n), s32)
#define C64(n) take64(n FOR_EACH_12(n), s64)
#define D12 ,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0
#define FOR_EACH_12(n) CHOOSE(n)
#define CHOOSE(n) C##n
#define C0
#define C1 ,1.0
#define C2 ,1.0,1.0
#define C3 ,1.0,1.0,1.0
#define C4 ,1.0,1.0,1.0,1.0
#define C5 ,1.0,1.0,1.0,1.0,1.0
#define C6 ,1.0,1.0,1.0,1.0,1.0,1.0
#define C7 ,1.0,1.0,1.0,1.0,1.0,1.0,1.0
#define C8 ,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0
#define C9 ,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0
#define C10 ,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0
#define C11 ,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0
#define C12 ,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0
int main(int argc, char **argv) {
    C32(0); C32(1); C32(2); C32(3); C32(4); C32(5); C32(6);
    C32(7); C32(8); C32(9); C32(10); C32(11); C32(12);
    C64(0); C64(1); C64(2); C64(3); C64(4); C64(5); C64(6);
    C64(7); C64(8); C64(9); C64(10); C64(11); C64(12);
    /* VLA (DynAlloca) perturbs %rsp before the realigned call */
    unsigned char buf[argc * 3 + 1];
    buf[0] = 1;
    C32(7);
    if (buf[0] != 1) return 9;
    /* wide parity sweep + integer-only overaligned struct */
    take64(19, 1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0, s64);
    take64(20, 1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0, s64);
    takeM(0, s32m);
    takeM(3, 3, 3, 3, s32m);
    takeM(9, 3,3,3,3,3,3,3,3,3, s32m);
    /* named overaligned stack param */
    struct N32 nn = { 6, 7 };
    if (named32(2, nn, 102) != 108.0) return 8;
    puts("va-interop PASS");
    return 0;
}
