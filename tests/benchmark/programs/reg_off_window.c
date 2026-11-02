/*
 * Fold-relevant microbenchmark: `window[cur + i]` with a register offset.
 *
 * The inner loop derives `p = window + (base ^ (i & 7))` and reads `p[j]` /
 * `p[k]` through VARIABLE indices, so the derived pointer's uses are indexed
 * GEPs and nothing else — exactly the shape `check_reg_off_sym_fold.sh`
 * proves is folded into `window(%off,%idx)` and, with the kill switch,
 * materialised with a LEA.  The workload is loop-bound on purpose: the
 * measurement is a Callgrind instruction-count A/B (never a wall-clock
 * claim), and the benchmark prints a checksum so both builds must agree.
 *
 * Usage:  lccc -O2 -march=x86-64-v3 -fno-pie -no-pie reg_off_window.c -o b
 *         env CCC_NO_REG_OFF_SYM=1 lccc ... (kill-switch build)
 *         valgrind --tool=callgrind --callgrind-out-file=... ./b
 */
#include <stdio.h>

unsigned char window[65536];

int
main(void)
{
    for (unsigned i = 0; i < 65536; i++)
        window[i] = (unsigned char)(i * 31 + 7);
    unsigned long acc = 0;
    for (unsigned pass = 0; pass < 3000; pass++) {
        for (unsigned i = 0; i < 512; i++) {
            /* The offset is computed as a value (`pass ^ (i & 7)`), so the
               derived pointer is `Add(window, t)` and its only uses are the
               two VARIABLE-index accesses: exactly the shape the fold is
               allowed to consume.  Writing it as `window + pass + i` let the
               front end fold the offset into the indices instead (no `Add`
               left to consume), which is why this spelling is deliberate. */
            const unsigned char *p = window + ((pass ^ (i & 7)) & 0x7fff);
            unsigned j = (i * 7) & 255;
            unsigned k = (i * 13) & 255;
            acc += p[j] + p[k];
        }
    }
    printf("%lu\n", acc);
    return 0;
}
