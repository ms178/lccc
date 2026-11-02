/* Shared setup for the __seg_gs kernels (Linux x86-64 only).
 *
 * A segment object at address A is reached as %gs:A. The kernel maps a mirror
 * of the object's pages at A + SEG_GS_DISP and points the %gs base at
 * SEG_GS_DISP, so %gs:A lands in the mirror and the setup code can write the
 * same bytes through an ordinary pointer. Only the segment kernels call this;
 * the shared driver and the generic kernels never touch %gs. */
#ifndef LCCC_BENCH_SEG_GS_H
#define LCCC_BENCH_SEG_GS_H

#include <errno.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <sys/syscall.h>
#include <unistd.h>

#define SEG_GS_ARCH_SET_GS 0x1001
#define SEG_GS_DISP ((uintptr_t)1 << 32)
/* Integer arithmetic: the mirror is 4 GiB past the object, which GCC would
 * otherwise diagnose as an out-of-bounds subscript. */
#define SEG_GS_MIRROR(g) ((void *)((uintptr_t)(g) + SEG_GS_DISP))

/* Map the mirror pages covering [g, g + n), one page at a time. Call this
 * before writing any segment data: the MAP_FIXED fallback below replaces a page
 * that is already mapped, which would zero its contents.
 *
 * A page already mapped by an earlier object fails with EEXIST and is skipped.
 * Valgrind ignores MAP_FIXED_NOREPLACE and returns another address instead, so
 * that case is undone and retried with MAP_FIXED. */
static inline void seg_gs_map(const void *g, size_t n) {
    uintptr_t lo = (uintptr_t)g + SEG_GS_DISP;
    uintptr_t end = lo + n;
    for (uintptr_t page = lo & ~(uintptr_t)4095; page < end; page += 4096) {
        void *want = (void *)page;
        void *m = mmap(want, 4096, PROT_READ | PROT_WRITE,
                       MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED_NOREPLACE, -1, 0);
        if (m == MAP_FAILED) {
            if (errno == EEXIST) continue;
            perror("mmap");
            exit(2);
        }
        if (m != want) {
            munmap(m, 4096);
            m = mmap(want, 4096, PROT_READ | PROT_WRITE,
                     MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, -1, 0);
            if (m != want) {
                perror("mmap(MAP_FIXED)");
                exit(2);
            }
        }
    }
}

static inline void seg_gs_enable(void) {
    if (syscall(SYS_arch_prctl, SEG_GS_ARCH_SET_GS, (void *)SEG_GS_DISP) != 0) {
        perror("arch_prctl(ARCH_SET_GS)");
        exit(2);
    }
}

#endif
