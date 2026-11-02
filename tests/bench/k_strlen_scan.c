/* Byte-walk scan loop: `while (*p) p++` over a 4095-byte string.
 *
 * This is the shape every `strlen`/`strchr`/`memchr`-style walk in gzip,
 * glibc and Expat compiles to after the test is folded, and it is the loop the
 * compiler either gets right or loses ~16% on:
 *
 *   top-tested (lccc):                    rotated (GCC 14.2 -O2):
 *     cmpb $0, (%r8)                        addq $1, %r8
 *     je   .Lexit                           cmpb $0, (%r8)
 *     addq $1, %r8                          jne  .Lloop
 *     jmp  .Lloop
 *
 * Measured on this host (31 interleaved samples, equal checksums): the
 * top-tested form 49.3 ms, the rotated form 59.2 ms for the same work, and GCC
 * emits the rotated form -- i.e. this kernel is where lccc is 15.9% *faster*
 * than GCC 14.2 on a byte-at-a-time scan, and any transform that rotates it
 * (or unrolls it badly) has to beat that number, not merely be defensible.
 * See docs/SESSION_FOLLOWUP_S21_LOOP_ROTATION.md section 5.
 *
 * The buffer deliberately contains no interior zero, so every scan walks all
 * 4095 bytes: the measurement is the steady-state loop, not the early exit.
 */
#include "bench.h"

#define N 4096

static unsigned char buf[N];

void bench_setup(void) {
    for (int i = 0; i < N; i++) {
        buf[i] = (unsigned char)(i % 251 + 1);
    }
    buf[N - 1] = 0;
}

static const unsigned char *scan_to_zero(const unsigned char *p) {
    while (*p) {
        p++;
    }
    return p;
}

unsigned long long bench_run(void) {
    unsigned long long s = 0;
    for (int i = 0; i < 32; i++) {
        s += (unsigned long)(scan_to_zero(buf) - buf);
    }
    return s;
}
