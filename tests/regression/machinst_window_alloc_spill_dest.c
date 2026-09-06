/* MachInst window-allocator regression: register pressure forces the main
 * allocator to spill BinOp destinations, which the window register allocator
 * must home (reload/operate/store) instead of replaying the whole window
 * through the accumulator text path. Loop-gate-independent: the loop body is
 * large (69 instructions), so before the 2026-09-06 gate raise this function
 * ran entirely on the mature path; with the allocator it runs MachInst
 * end-to-end. GCC is the oracle (stdout + exit code). */
#include <stdio.h>

long window_alloc_pressure(const long *in, long n) {
    long a0 = in[0], a1 = in[1], a2 = in[2], a3 = in[3];
    long a4 = in[4], a5 = in[5], a6 = in[6], a7 = in[7];
    long a8 = in[8], a9 = in[9], a10 = in[10], a11 = in[11];
    long a12 = in[12], a13 = in[13], a14 = in[14], a15 = in[15];
    long s = 0;
    for (long i = 0; i < n; i++) {
        s ^= a0 + i;  a0 += 3;
        s += a1 - i;  a1 ^= 5;
        s -= a2 * 3;  a2 += 7;
        s ^= a3 + i;  a3 -= 11;
        s += a4 * 5;  a4 ^= 13;
        s -= a5 + i;  a5 += 17;
        s ^= a6 - i;  a6 *= 3;
        s += a7 * 7;  a7 += 19;
        s -= a8 + i;  a8 ^= 23;
        s ^= a9 * 3;  a9 += 29;
        s += a10 - i; a10 ^= 31;
        s -= a11 * 5; a11 += 37;
        s ^= a12 + i; a12 -= 41;
        s += a13 * 9; a13 ^= 43;
        s -= a14 - i; a14 += 47;
        s ^= a15 + i; a15 *= 3;
    }
    return s + a0 + a1 + a2 + a3 + a4 + a5 + a6 + a7
             + a8 + a9 + a10 + a11 + a12 + a13 + a14 + a15;
}

int main(void) {
    long in[16] = {10,20,30,40,50,60,70,80,90,100,110,120,130,140,150,160};
    long r = window_alloc_pressure(in, 3);
    printf("%ld\n", r);
    /* A second, differently-shaped round catches width/signedness drift. */
    long in2[16] = {-1,-2,-3,-4,-5,-6,-7,-8,-9,-10,-11,-12,-13,-14,-15,-16};
    printf("%ld\n", window_alloc_pressure(in2, 4));
    return 0;
}
