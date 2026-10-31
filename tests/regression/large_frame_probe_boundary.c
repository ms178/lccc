// A frame that crosses the x86-64 stack-clash probe page, and is EXECUTED.
//
// large_stack_frame_codegen.c pins the three *encoding* ranges of a stack
// adjustment, but its frames are 2 GiB and 3 GiB, so nothing can call it: it is
// a compile-and-unwind test. The adjustment that crosses a 4 KiB page is the
// one shape a program can run, and it needs `subq` to be split into probe steps
// -- the guard-page-sized decrements a stack-clash attack relies on the OS to
// catch. Getting that split wrong (a plain 12 KiB `subq`, or a probe loop that
// subtracts one page too many) corrupts the frame silently: the function
// returns, having written outside it.
//
// Everything here is volatile and every page of the frame is touched BEFORE
// and AFTER the call, so a clobbered guard page or a one-page-short frame is a
// wrong sum rather than an undetected success.
#include <stdio.h>

#define FRAME_PAGES 3

__attribute__((noinline)) static long probe_boundary(int seed) {
    // Volatile so the array is a real frame object with a real initializer,
    // not a memset the optimizer can fold away.
    volatile unsigned char buf[FRAME_PAGES * 4096];
    long sum = 0;
    for (int page = 0; page < FRAME_PAGES; page++) {
        buf[page * 4096] = (unsigned char)(seed + page);
        buf[page * 4096 + 4095] = (unsigned char)(seed - page);
    }
    // A call out of the probed region: the callee's frame has to land below
    // the LAST probe, not below the first one.
    for (int page = 0; page < FRAME_PAGES; page++) {
        sum += buf[page * 4096];
        sum += buf[page * 4096 + 4095];
        sum += (long)page * 2;
    }
    return sum;
}

int main(void) {
    long total = 0;
    for (int seed = 0; seed < 17; seed++) {
        total += probe_boundary(seed);
    }
    // Deterministic and reproducible by any correct compiler: 17 seeds x
    // (sum of the two touched bytes per page x 3 pages) + the page-index term.
    printf("probe_boundary total=%ld\n", total);
    return total == 0 ? 1 : 0;
}
