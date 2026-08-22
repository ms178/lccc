/* Regression: loop IV must not ride a caller-saved register across a call.
 *
 * Shape (zlib-ng minideflate / CVE-2018-25032):
 *
 *     for (int i = 1; i < argc; i++)
 *         if (strcmp(argv[i], "-m") == 0 && i + 1 < argc)
 *             mem_level = atoi(argv[++i]);
 *
 * `++i` is both the argv index of the call and the latch incoming.  A
 * destructive phi-coalesce that homes that increment in %edi is clobbered
 * by strtol; the shared latch then increments garbage and later flags
 * (-w/-s/-F) are never seen.
 *
 *     expected  "1 -15 2 6 0 0 4 1 1 1"
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int parse(int argc, char **argv) {
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
    return mem_level == 1 && window_bits == -15 && strategy == 2 && level == 6
           && rbs == 0 && wbs == 0 && flush == 4 && copyout == 1 && uncompr == 1 && keep == 1
           ? 0 : 1;
}

int main(void) {
    char *argv[] = {
        "t", "-c", "-k", "-d", "-m", "1", "-w", "-15", "-s", "4", "-F", "-6", NULL
    };
    return parse(12, argv);
}
