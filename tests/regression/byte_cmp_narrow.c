/* PF-15: single-use widening byte/half casts feeding an integer compare
 * must fold into one narrow cmpb/cmpw on the ORIGINAL sources (GCC's
 * exact shape) whenever the extension/opcode compatibility matrix allows
 * — sext pairs keep the signed setcc family, zext pairs map every opcode
 * onto the unsigned family, mixed extensions and out-of-domain constants
 * are refused (materializing both widening moves instead). The harness
 * compares program output against GCC, so any wrong flag mapping or a
 * missing deferred widening move shows up as a mismatch on some input
 * combination. Exhaustive over the byte domains; strided over the half
 * domains. */
#include <stdio.h>

static volatile int v_zero = 0;

int scmp(signed char a, signed char b) { return a < b; }
int sge(signed char a, signed char b) { return a >= b; }
int seq(signed char a, signed char b) { return a == b; }
int ucmp(unsigned char a, unsigned char b) { return a < b; }
int uge(unsigned char a, unsigned char b) { return a >= b; }
int ub_mix(unsigned char a, signed char b) { return a < b; }
int slt16(short a, short b) { return a < b; }
int ule16(unsigned short a, unsigned short b) { return a <= b; }
int c_sext(signed char a) { return a < 100; }
int c_zext(unsigned char a) { return a >= 200; }
int c_sixteen(unsigned short a) { return a == 65535; }

int main(void) {
    int fails = 0;
    for (int a = -128; a <= 127; a++) {
        for (int b = -128; b <= 127; b++) {
            unsigned char ua = (unsigned char)a;
            unsigned char ub = (unsigned char)b;
            if (scmp(a, b) != (a < b)) fails++;
            if (sge(a, b) != (a >= b)) fails++;
            if (seq(a, b) != (a == b)) fails++;
            if (ucmp(ua, ub) != (ua < ub)) fails++;
            if (uge(ua, ub) != (ua >= ub)) fails++;
            if (ub_mix(ua, b) != (ua < b)) fails++;
            if (c_sext(a) != (a < 100)) fails++;
            if (c_zext(ua) != (ua >= 200)) fails++;
            if (fails > 40) { printf("FAILS %d\n", fails); return 1; }
        }
    }
    for (int a = -32768; a <= 32767; a += 7) {
        for (int b = -32768; b <= 32767; b += 11) {
            unsigned short ua = (unsigned short)a;
            unsigned short ub = (unsigned short)b;
            if (slt16(a, b) != (a < b)) fails++;
            if (ule16(ua, ub) != (ua <= ub)) fails++;
            if (c_sixteen(ua) != (ua == 65535)) fails++;
            if (fails > 40) { printf("FAILS %d\n", fails); return 1; }
        }
    }
    /* Deferred-widen interaction with a volatile sink: the loads around
     * the compare must not reorder the widening moves' liveness. */
    if (v_zero == 0) {
        signed char s = -1;
        unsigned char u = 1;
        if (!(scmp(s, 0) == 1 && ucmp(u, 0) == 0 && ub_mix(u, s) == 0)) fails++;
    }
    if (fails != 0) { printf("FAILS %d\n", fails); return 1; }
    printf("byte-cmp-narrow OK\n");
    return 0;
}
