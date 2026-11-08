/* Allocation-count interposer: a deterministic compile-cost proxy.
 *
 * WHY THIS EXISTS
 * ---------------
 * MB-20 asks for a compile-cost ratchet with a deterministic proxy. Callgrind
 * is the usual answer, but it is not always installed, and wall time on a
 * small shared host cannot resolve a few percent. For an "allocate before
 * filtering" optimization the number of allocations IS the quantity being
 * removed, so counting them is both cheaper and more direct than counting
 * instructions -- and it is exactly reproducible.
 *
 * It interposes the glibc entry points Rust's System allocator uses (malloc,
 * calloc, realloc, free, posix_memalign, aligned_alloc) and writes totals at
 * exit to $ALLOC_COUNT_OUT.<pid>.
 *
 * THREE MEASUREMENT TRAPS THIS FILE EXISTS TO AVOID
 * -----------------------------------------------
 * Each of these was hit while building the gate; each silently produced a
 * plausible-looking but WRONG number, which is worse than no number:
 *
 *  1. Thread-local counters lose worker threads. lccc compiles on worker
 *     threads, so a __thread counter folded by a main-thread destructor
 *     reported 15 allocations for a compile that really performs ~29,600.
 *     The counters here are GLOBAL RELAXED atomics: every thread is counted.
 *     Relaxed ordering suffices because the total is read only at exit,
 *     after the workers have joined.
 *
 *  2. A fixed output path loses child processes. If the driver spawns a
 *     child, parent and child race for the same file and the smaller count
 *     can win. The pid suffix keeps every process's report; the harness sums
 *     them.
 *
 *  3. The reported BYTE total depends on the environment. std::env copies
 *     the environment, so a longer $ALLOC_COUNT_OUT path measurably changes
 *     `bytes`. The harness therefore passes an identical path to both arms.
 *     Allocation COUNT is unaffected, which is why count is the gated
 *     metric and bytes is reported only as supporting evidence.
 *
 * The interposer never allocates: counters are plain integers and the exit
 * path uses open/write/close directly, so there is no recursion into malloc.
 */
#define _GNU_SOURCE
#include <fcntl.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>

/* glibc exports these but no longer declares them in public headers. */
extern void *__libc_malloc(size_t);
extern void *__libc_calloc(size_t, size_t);
extern void *__libc_realloc(void *, size_t);
extern void __libc_free(void *);
extern int __libc_posix_memalign(void **, size_t, size_t);
extern void *__libc_aligned_alloc(size_t, size_t);

static uint64_t n_malloc, n_calloc, n_realloc, n_free, n_aligned, n_bytes;
#define bump(v, by) __atomic_add_fetch(&(v), (by), __ATOMIC_RELAXED)

void *malloc(size_t s) {
    bump(n_malloc, 1);
    bump(n_bytes, s);
    return __libc_malloc(s);
}

void *calloc(size_t n, size_t s) {
    bump(n_calloc, 1);
    bump(n_bytes, n * s);
    return __libc_calloc(n, s);
}

void *realloc(void *p, size_t s) {
    bump(n_realloc, 1);
    bump(n_bytes, s);
    return __libc_realloc(p, s);
}

void free(void *p) {
    if (p) bump(n_free, 1);
    __libc_free(p);
}

int posix_memalign(void **out, size_t a, size_t s) {
    bump(n_aligned, 1);
    bump(n_bytes, s);
    return __libc_posix_memalign(out, a, s);
}

void *aligned_alloc(size_t a, size_t s) {
    bump(n_aligned, 1);
    bump(n_bytes, s);
    return __libc_aligned_alloc(a, s);
}

__attribute__((destructor)) static void report(void) {
    const char *path = getenv("ALLOC_COUNT_OUT");
    char pbuf[4096];
    if (path) {
        snprintf(pbuf, sizeof pbuf, "%s.%d", path, (int)getpid());
        path = pbuf;
    }
    char buf[256];
    int len = snprintf(
        buf, sizeof buf,
        "malloc=%llu calloc=%llu realloc=%llu free=%llu aligned=%llu bytes=%llu total=%llu\n",
        (unsigned long long)n_malloc, (unsigned long long)n_calloc,
        (unsigned long long)n_realloc, (unsigned long long)n_free,
        (unsigned long long)n_aligned, (unsigned long long)n_bytes,
        (unsigned long long)(n_malloc + n_calloc + n_realloc + n_aligned));
    if (len <= 0) return;
    int fd = path ? open(path, O_WRONLY | O_CREAT | O_TRUNC, 0644) : 2;
    if (fd >= 0) {
        ssize_t ignored = write(fd, buf, (size_t)len);
        (void)ignored;
        if (path) close(fd);
    }
}
