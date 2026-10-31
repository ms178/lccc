/* Fault injection for the scoreboard's error path; never linked in benchmarks. */
#include <errno.h>
#include <time.h>

int clock_gettime(clockid_t clock_id, struct timespec *result) {
    (void)clock_id;
    (void)result;
    errno = EIO;
    return -1;
}
