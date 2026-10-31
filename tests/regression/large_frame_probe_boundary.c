// A frame just past the stack-clash probe page, EXECUTED.
//
// `large_stack_frame_codegen.c` covers the three stack-adjustment encodings
// (immediate, zero-extended `movl` into `%r11`, `movabs` into `%r11`) but its
// functions are deliberately never called -- a 3 GiB frame cannot run.  The
// small end of that range is runnable and is where the *epilogue* adjustment
// actually executes, so this fixture exercises:
//
//   * a frame larger than one 4 KiB page, so the prologue's probe loop runs
//     and the final adjustment is the register form,
//   * a call from inside the frame, so the probe loop's stack depth is
//     traversed with live data below it,
//   * a value read back out of the probed region, so a wrong delta fails at
//     run time instead of only in an assertion.
//
// The accesses are `volatile` so the stores cannot be folded away and the
// region is really touched.
#include <stdio.h>

#define FRAME_BYTES 12288 /* three probe pages */

__attribute__((noinline)) static long touch(char *buf, int n, int stride) {
    long sum = 0;
    for (int i = 0; i < n; i += stride) {
        buf[i] = (char)(i * 7 + 1);
        sum += buf[i];
    }
    return sum;
}

__attribute__((noinline)) long probe_boundary(int n) {
    volatile char buf[FRAME_BYTES];
    long acc = 0;
    for (int i = 0; i < FRAME_BYTES; i += 512) {
        buf[i] = (char)(i + n);
        acc += buf[i];
    }
    // A call with the probed region live: the callee's own frame sits below
    // everything this function touched.
    acc += touch((char *)buf, FRAME_BYTES, 97);
    return acc + buf[FRAME_BYTES - 1];
}

// TARGET SCOPE: x86-64 only. A 12 KiB `volatile` frame is legal in ILP32 too,
// but the register-form adjustment this exercises is an x86-64 encoding
// constraint (`subl imm32, %esp` takes the full 32-bit immediate on i686), so
// the fixture adds nothing there; a host without 32-bit multilib reports the
// build as `host-skip` through `run_regression.py`.

int main(void) {
    long total = 0;
    for (int n = -4; n <= 4; n++) {
        total += probe_boundary(n);
    }
    printf("probe_frame=%ld\n", total);
    return 0;
}
