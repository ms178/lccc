/* Guard-page test: aggregate transfers of 1..15 bytes must touch exactly the
 * bytes of the object.
 *
 * Each object ends at the end of a readable page, and the following page is
 * PROT_NONE. A load or store one byte too wide faults. The same objects are
 * exercised through a __seg_gs view (GS base = the page) and through an
 * ordinary generic pointer, for:
 *   - returns (sizes 1..7: single-register class; 9..15: two-register class),
 *   - by-value arguments,
 *   - whole-object stores from a register-class value.
 * A fault is reported as a FAIL line by the SIGSEGV handler and exits 1.
 *
 * Output: one "ok" line per check and a final "PASSED (0 failures)" line. */
#define _GNU_SOURCE
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/syscall.h>
#include <unistd.h>

#define ARCH_SET_GS 0x1001
#define PAGE 4096UL

static int failures;

static void on_fault(int sig) {
    (void)sig;
    static const char msg[] = "FAIL: access past the object (guard page hit)\n";
    ssize_t r = write(2, msg, sizeof msg - 1);
    (void)r;
    _exit(1);
}

static void check(int ok, const char *name) {
    if (ok) {
        printf("ok %s\n", name);
    } else {
        printf("not ok %s\n", name);
        failures++;
    }
}

#define DEFINE_SIZE(N)                                                              \
    typedef struct { unsigned char b[N]; } __attribute__((packed)) s##N;           \
    static s##N __seg_gs *g##N;                                                    \
    static s##N *n##N;                                                             \
    __attribute__((noinline)) static s##N ret_seg##N(void) { return *g##N; }       \
    __attribute__((noinline)) static s##N ret_gen##N(void) { return *n##N; }       \
    __attribute__((noinline)) static long take##N(s##N v) {                        \
        long t = 0;                                                                \
        for (int i = 0; i < N; i++) t += (long)v.b[i] * (i + 1);                   \
        return t;                                                                  \
    }                                                                              \
    __attribute__((noinline)) static void put_seg##N(s##N v) { *g##N = v; }        \
    __attribute__((noinline)) static void put_gen##N(s##N v) { *n##N = v; }        \
    static void run##N(void) {                                                     \
        unsigned char want[N];                                                     \
        long expect = 0;                                                           \
        for (int i = 0; i < N; i++) {                                              \
            want[i] = (unsigned char)(0x10 + i * 7);                               \
            expect += (long)want[i] * (i + 1);                                     \
        }                                                                          \
        s##N v;                                                                    \
        memcpy(&v, want, N);                                                       \
        /* Seed both views through the write paths under test. */                  \
        put_seg##N(v);                                                             \
        s##N r = ret_seg##N();                                                     \
        char name[64];                                                             \
        snprintf(name, sizeof name, "size%d seg return", N);                       \
        check(memcmp(&r, want, N) == 0, name);                                     \
        snprintf(name, sizeof name, "size%d seg argument", N);                     \
        check(take##N(*g##N) == expect, name);                                     \
        put_gen##N(v);                                                             \
        r = ret_gen##N();                                                          \
        snprintf(name, sizeof name, "size%d generic return", N);                   \
        check(memcmp(&r, want, N) == 0, name);                                     \
        snprintf(name, sizeof name, "size%d generic argument", N);                 \
        check(take##N(*n##N) == expect, name);                                     \
    }

DEFINE_SIZE(1)
DEFINE_SIZE(2)
DEFINE_SIZE(3)
DEFINE_SIZE(4)
DEFINE_SIZE(5)
DEFINE_SIZE(6)
DEFINE_SIZE(7)
DEFINE_SIZE(9)
DEFINE_SIZE(10)
DEFINE_SIZE(11)
DEFINE_SIZE(12)
DEFINE_SIZE(13)
DEFINE_SIZE(14)
DEFINE_SIZE(15)

#define RUN(N) run##N()

int main(void) {
    /* Segment view: two pages, the second one inaccessible. */
    unsigned char *seg = mmap(NULL, 2 * PAGE, PROT_READ | PROT_WRITE,
                              MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    /* Generic view: a separate pair of pages with the same layout. */
    unsigned char *gen = mmap(NULL, 2 * PAGE, PROT_READ | PROT_WRITE,
                              MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (seg == MAP_FAILED || gen == MAP_FAILED) {
        perror("mmap");
        return 2;
    }
    if (mprotect(seg + PAGE, PAGE, PROT_NONE) != 0 ||
        mprotect(gen + PAGE, PAGE, PROT_NONE) != 0) {
        perror("mprotect");
        return 2;
    }
    if (syscall(SYS_arch_prctl, ARCH_SET_GS, seg) != 0) {
        perror("arch_prctl");
        return 2;
    }
    setvbuf(stdout, NULL, _IONBF, 0); /* the fault handler _exits: no lost lines */
    signal(SIGSEGV, on_fault);
    signal(SIGBUS, on_fault);

#define BIND(N)                                                                     \
    g##N = (s##N __seg_gs *)(unsigned long)(PAGE - N);                              \
    n##N = (s##N *)(void *)(gen + PAGE - N);
    BIND(1) BIND(2) BIND(3) BIND(4) BIND(5) BIND(6) BIND(7)
    BIND(9) BIND(10) BIND(11) BIND(12) BIND(13) BIND(14) BIND(15)

    RUN(1); RUN(2); RUN(3); RUN(4); RUN(5); RUN(6); RUN(7);
    RUN(9); RUN(10); RUN(11); RUN(12); RUN(13); RUN(14); RUN(15);

    if (failures) {
        printf("FAILED (%d failures)\n", failures);
        return 1;
    }
    printf("PASSED (0 failures)\n");
    return 0;
}
