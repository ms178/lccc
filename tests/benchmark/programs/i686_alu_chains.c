/* i686 ALU latency-chain micro-benchmark (A/B for src/backend/i686/codegen/alu.rs).
 *
 * Each kernel is a loop-carried dependency chain through ONE lowering, so the
 * measured time is the latency of the generated sequence (the quantity the
 * constant-division / multiply strength reductions optimise), not throughput
 * or memory behaviour.  Every kernel folds its result into a checksum that is
 * printed, so two compilers (or two revisions) must agree bit-for-bit.
 *
 * Build/run (32-bit):
 *   lccc-i686 -O2 -o chains i686_alu_chains.c && ./chains [iters]
 *   gcc -m32 -O2 -o chains.gcc i686_alu_chains.c && ./chains.gcc
 *
 * Output: one line per kernel  "<name> <checksum> <ns/iter>"  and a final
 * "TOTAL <checksum>" line.  Use the min of several runs.
 */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <time.h>

#define NI __attribute__((noinline))
typedef uint32_t u32;
typedef int32_t s32;

static double now_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec * 1e9 + ts.tv_nsec;
}

/* ---- kernels: x is the chain, i decorrelates the values ---------------- */
NI u32 k_urem7(u32 x, u32 n) { for (u32 i = 0; i < n; i++) x = (x % 7u) + i; return x; }
NI u32 k_urem10(u32 x, u32 n) { for (u32 i = 0; i < n; i++) x = (x % 10u) + (i ^ x); return x; }
NI u32 k_udiv7(u32 x, u32 n) { for (u32 i = 0; i < n; i++) x = (x / 7u) + (i * 3u); return x; }
NI u32 k_udr10(u32 x, u32 n) { for (u32 i = 0; i < n; i++) { u32 q = x / 10u, r = x % 10u; x = q + r * 3u + i; } return x; }
NI u32 k_sdr7(s32 x, u32 n) { for (u32 i = 0; i < n; i++) { s32 q = x / 7, r = x % 7; x = q + r * 5 - (s32)i; } return (u32)x; }
NI u32 k_srem7(s32 x, u32 n) { for (u32 i = 0; i < n; i++) x = (x % 7) + (s32)(i & 0xffff) - 30000; return (u32)x; }
NI u32 k_srem16(s32 x, u32 n) { for (u32 i = 0; i < n; i++) x = (x % 16) * 3 + (s32)(i & 0xff) - 100; return (u32)x; }
NI u32 k_sdr16(s32 x, u32 n) { for (u32 i = 0; i < n; i++) { s32 q = x / 16, r = x % 16; x = q ^ (r << 3) ^ (s32)i; } return (u32)x; }
NI u32 k_mul7(u32 x, u32 n) { for (u32 i = 0; i < n; i++) x = (x * 7u) ^ i; return x; }
NI u32 k_mul11(u32 x, u32 n) { for (u32 i = 0; i < n; i++) x = (x * 11u) ^ i; return x; }
NI u32 k_mul17(u32 x, u32 n) { for (u32 i = 0; i < n; i++) x = (x * 17u) ^ i; return x; }
NI u32 k_mul24(u32 x, u32 n) { for (u32 i = 0; i < n; i++) x = (x * 24u) ^ i; return x; }
NI u32 k_mul45(u32 x, u32 n) { for (u32 i = 0; i < n; i++) x = (x * 45u) ^ i; return x; }
NI u32 k_mulm3(u32 x, u32 n) { for (u32 i = 0; i < n; i++) x = (x * (u32)-3) ^ i; return x; }
NI u32 k_mul1000(u32 x, u32 n) { for (u32 i = 0; i < n; i++) x = (x * 1000u) ^ i; return x; } /* stays imull */
NI u32 k_fnv(u32 x, u32 n) { for (u32 i = 0; i < n; i++) { x ^= i & 0xff; x *= 16777619u; } return x; }
NI u32 k_digits(u32 x, u32 n) { /* itoa-style: digit peel */
    u32 acc = 0;
    for (u32 i = 0; i < n; i++) { u32 v = x + i; while (v) { acc += v % 10u; v /= 10u; } }
    return acc;
}

typedef u32 (*kernel_fn)(u32, u32);
struct kernel { const char *name; kernel_fn fn; u32 seed; };

int main(int argc, char **argv) {
    u32 iters = argc > 1 ? (u32)strtoul(argv[1], 0, 0) : 20000000u;
    const struct kernel ks[] = {
        {"urem7", k_urem7, 123456789u}, {"urem10", k_urem10, 987654321u},
        {"udiv7", k_udiv7, 0xdeadbeefu}, {"udr10", k_udr10, 0x12345678u},
        {"sdr7", (kernel_fn)k_sdr7, 0x7fff1234u}, {"srem7", (kernel_fn)k_srem7, 0x80001234u},
        {"srem16", (kernel_fn)k_srem16, 0xfffffff0u}, {"sdr16", (kernel_fn)k_sdr16, 0x0fedcba9u},
        {"mul7", k_mul7, 1u}, {"mul11", k_mul11, 3u}, {"mul17", k_mul17, 5u},
        {"mul24", k_mul24, 7u}, {"mul45", k_mul45, 9u}, {"mulm3", k_mulm3, 11u},
        {"mul1000", k_mul1000, 13u}, {"fnv", k_fnv, 2166136261u},
        {"digits", k_digits, 1234567u},
    };
    u32 total = 0;
    for (unsigned k = 0; k < sizeof ks / sizeof ks[0]; k++) {
        u32 n = ks[k].fn == k_digits ? iters / 16 : iters;
        double t0 = now_ns();
        u32 r = ks[k].fn(ks[k].seed, n);
        double t1 = now_ns();
        total = total * 31u + r;
        /* Checksums on stdout (the byte-compared regression signal);
         * wall-clock per iteration on stderr — it differs on every run and
         * across compilers, which used to flip every stdout comparison into
         * a coin toss (regression suite + peephole A/B gate). */
        fprintf(stderr, "%-8s %11.3f\n", ks[k].name, (t1 - t0) / n);
        printf("%-8s %08x\n", ks[k].name, r);
    }
    printf("TOTAL %08x\n", total);
    return 0;
}
