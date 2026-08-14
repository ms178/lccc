/*
 * v12 regression: dead parameter store elimination (v11).
 *
 * A parameter that is constant-propagated away at the call site, or a
 * parameter that is simply unused by the body, must not corrupt the values
 * that ARE used. Covers unused/constant FP and int params, and the
 * interaction with multiple params sharing the frame.
 */
#include <stdio.h>

__attribute__((noinline)) static int use_one(int used, int unused) { return used * 3; }
__attribute__((noinline)) static int use_none(int a, int b) { (void)a; (void)b; return 42; }
__attribute__((noinline)) static double constfp(double unused_scale) { (void)unused_scale; return 7.5; }
__attribute__((noinline)) static double foldfp(double dt) { return dt * 2.0; }  /* call site passes a constant */
__attribute__((noinline)) static int mixed(int a, double x, int b) { return a + b + (int)x; }

int main(void) {
    int r = 0, i;
    for (i = 0; i < 100000; i++) {
        r += use_one(i, i + 1000);
        r += use_none(i, i);
        r += (int)constfp(123.0);
        r += (int)foldfp(0.5);      /* constant argument */
        r += mixed(i, 2.5, i + 1);
    }
    printf("%d\n", r);
    return 0;
}
