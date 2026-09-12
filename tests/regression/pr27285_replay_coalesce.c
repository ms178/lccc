/* Regression (GCC PR tree-optimization/27285, reduced): a loop-carried value
   feeding a compare with two if-converted Select consumers must not be
   compare-replay-pruned when RA coalesces the latch copy onto the same home.
   The second select's replay would re-emit the compare after the first select
   clobbered the shared home, tripping the stale-home ICE. The home-collision
   veto in the prologue replay scan must keep the second replay. */
extern void abort(void);

static unsigned foo(unsigned b) {
    unsigned c = 0, acc = 0;
    while (b) {
        if (b >= 8) {
            c = 0xff;
            b -= 8;
        } else {
            c = 0xffu << (8 - b);
            b = 0;
        }
        acc += c;
    }
    return acc;
}

int main(void) {
    /* 25->17->9->1->0: 255+255+255+32640 = 33405 */
    if (foo(25) != 33405)
        abort();
    return 0;
}
