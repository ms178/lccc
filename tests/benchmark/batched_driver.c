/* Common GCC-built infrastructure, no LTO. Each child starts with pristine
 * workload globals; fork/wait/exec startup is outside the measured interval.
 * Benchmark entry code, data and relocations are unchanged by symbol renaming.
 * Bounded Callgrind runs use the original executables, not this fork driver.
 */
#include <errno.h>
#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <sys/wait.h>
#include <unistd.h>
#include <time.h>
extern int benchmark_entry(void);
static uint64_t clock_ns(void) {
    struct timespec t;
    if (clock_gettime(CLOCK_MONOTONIC, &t)) _exit(98);
    return (uint64_t)t.tv_sec * UINT64_C(1000000000) + (uint64_t)t.tv_nsec;
}
int main(int argc, char **argv) {
    if (argc != 2) return 99;
    char *end; errno = 0;
    unsigned long n = strtoul(argv[1], &end, 10);
    if (errno || !*argv[1] || *end || *argv[1] == '-' || !n || n > 1000000) return 99;
    volatile uint64_t *elapsed = mmap(NULL, sizeof(*elapsed), PROT_READ | PROT_WRITE,
                                     MAP_SHARED | MAP_ANONYMOUS, -1, 0);
    if (elapsed == MAP_FAILED) return 98;
    uint64_t total = 0;
    for (unsigned long i = 0; i < n; ++i) {
        *elapsed = 0;
        pid_t child = fork();
        if (child < 0) return 98;
        if (child == 0) {
            uint64_t start = clock_ns();
            int result = benchmark_entry();
            *elapsed = clock_ns() - start;
            /* printf buffering is part of the workload. Flush outside timing,
             * before _exit (which deliberately avoids parent exit handlers). */
            if (fflush(stdout)) _exit(98);
            _exit(result ? 97 : 0);
        }
        int status;
        pid_t waited;
        do { waited = waitpid(child, &status, 0); } while (waited < 0 && errno == EINTR);
        if (waited != child || !WIFEXITED(status) || WEXITSTATUS(status) || !*elapsed) return 97;
        if (UINT64_MAX - total < *elapsed) return 98;
        total += *elapsed;
    }
    if (munmap((void *)elapsed, sizeof(*elapsed))) return 98;
    fprintf(stderr, "BATCH_RESULT {\"ns\":%" PRIu64 ",\"iterations\":%lu}\n", total, n);
    return 0;
}
