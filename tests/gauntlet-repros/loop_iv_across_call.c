/* GAUNTLET REPRODUCER / REGRESSION PIN.
 *
 * Fixed by refusing phi-coalesce register inheritance across a caller-saved
 * clobber (see detect/apply_phi_coalesce in regalloc.rs). Keep this file.
 *
 * Loop induction variable carried in a caller-saved register across a call.
 *
 * In
 *
 *     for (int i = 1; i < argc; i++) {
 *         if (strcmp(argv[i], "-m") == 0 && i + 1 < argc)
 *             mem_level = atoi(argv[++i]);
 *         ...
 *     }
 *
 * the `++i` produces an SSA value `i1 = i + 1` that is BOTH the array index
 * of `argv[++i]` and the loop induction value fed to the backedge (`i = i1;
 * i++`).  The x86 backend computes `i1` in %edi BEFORE the `strtol` call
 * (for the index), then the shared loop latch does `movl %edi,%eax;
 * addl $1,%eax` — but %edi is caller-saved and is clobbered by the call.
 * The latch therefore stores a garbage induction value and the loop runs
 * past `argc` into the environment array (observable as minideflate trying
 * to fopen("E2B_TEMPLATE_ID=...")).
 *
 * Evidence: zlib-ng 2.3.3 CTest `CVE-2018-25032` (6 variants) and `GH-382`
 * fail with "Failed to open file: <ENVVAR=...>" on BOTH the re-base point
 * and this tree; a GCC build of the same zlib-ng passes 100%.  This file
 * reproduces the defect standalone:
 *
 *     lccc -O2 loop_iv_across_call.c -o t && ./t -c -k -d -m 1 -w -15 -s 4 -F -6
 *     # expected  "1 -15 2 6 0 0 4 1 1 1"
 *     # observed  "1 0 0 6 0 0 0 1 1 1"  (window_bits/strategy/flush lost)
 *
 * The fix belongs in the backend latch/RA: a value live across a call must
 * never ride a caller-saved register, or the latch must reload the induction
 * slot instead of trusting %edi.  Do not delete this file when fixing — it is
 * the regression pin.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int main(int argc, char **argv) {
    int mem_level = 0, window_bits = 0, strategy = 0, level = 0, rbs = 0, wbs = 0, flush = 0;
    int copyout = 0, uncompr = 0, keep = 0;
    for (int i = 1; i < argc; i++) {
        if ((strcmp(argv[i], "-m") == 0) && (i + 1 < argc)) mem_level = atoi(argv[++i]);
        else if ((strcmp(argv[i], "-w") == 0) && (i + 1 < argc)) window_bits = atoi(argv[++i]);
        else if ((strcmp(argv[i], "-r") == 0) && (i + 1 < argc)) rbs = atoi(argv[++i]);
        else if ((strcmp(argv[i], "-t") == 0) && (i + 1 < argc)) wbs = atoi(argv[++i]);
        else if ((strcmp(argv[i], "-s") == 0) && (i + 1 < argc)) flush = atoi(argv[++i]);
        else if (strcmp(argv[i], "-c") == 0) copyout = 1;
        else if (strcmp(argv[i], "-d") == 0) uncompr = 1;
        else if (strcmp(argv[i], "-k") == 0) keep = 1;
        else if (strcmp(argv[i], "-f") == 0) strategy = 1;
        else if (strcmp(argv[i], "-F") == 0) strategy = 2;
        else if (strcmp(argv[i], "-h") == 0) strategy = 3;
        else if (strcmp(argv[i], "-R") == 0) strategy = 4;
        else if (argv[i][0] == '-' && argv[i][1] >= '0' && argv[i][1] <= '9' && argv[i][2] == 0)
            level = argv[i][1] - '0';
    }
    printf("%d %d %d %d %d %d %d %d %d %d\n",
           mem_level, window_bits, strategy, level, rbs, wbs, flush, copyout, uncompr, keep);
    return 0;
}
