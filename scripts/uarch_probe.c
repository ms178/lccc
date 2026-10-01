/* uarch_probe.c — measure dependency-breaking / latency facts on the HOST core.
 *
 * Build & run (scripts/uarch_probe.sh does this and pins the CPU):
 *     gcc -O2 -o /tmp/uarch_probe scripts/uarch_probe.c && taskset -c 1 /tmp/uarch_probe
 *
 * Why this exists: a VM exposes no PMU, and TSC ticks at a fixed rate that is
 * NOT the core clock.  Every figure is therefore printed *relative to a
 * dependent ADD chain* (1 cycle per link on every x86 core since the 386):
 *
 *     cycles/iter = (TSC ticks/iter of the probe) / (TSC ticks/link of ADD)
 *
 * which cancels the clock ratio, turbo and VM overhead.  The probe chains
 * use the standard latency-chain technique (Agner Fog / uops.info): a
 * long-latency producer (IMUL, 3 cycles) feeds the instruction under test,
 * whose destination is the next iteration's IMUL input.  If the instruction
 * architecturally reads its destination (a *false dependency*) the loop pays
 * IMUL + probe per iteration; if the hardware breaks the dependency the
 * iterations overlap and the loop runs at the issue-width bound.
 *
 * Verdict column: DEP  = per-iteration cost >= 2.5 cycles (chain through dest)
 *                 FREE = per-iteration cost <  2.5 cycles (dependency broken)
 */
#include <stdint.h>
#include <stdio.h>
#include <time.h>

static inline uint64_t tsc(void) {
    uint32_t lo, hi;
    __asm__ volatile("rdtsc" : "=a"(lo), "=d"(hi));
    return ((uint64_t)hi << 32) | lo;
}
#define N 20000000ull

#define PROBE2(name, body)                                                   \
    static double name(void) {                                               \
        uint64_t best = ~0ull;                                               \
        for (int rep = 0; rep < 7; rep++) {                                  \
            uint64_t n = N, a = 3, b = 0x123456789ull, c = 5, d = 7;         \
            uint64_t t0 = tsc();                                             \
            __asm__ volatile(body : [n] "+r"(n), [a] "+r"(a), [b] "+r"(b),   \
                                    [c] "+r"(c), [d] "+r"(d)::"cc");         \
            uint64_t t1 = tsc();                                             \
            if (t1 - t0 < best) best = t1 - t0;                              \
        }                                                                    \
        return (double)best / (double)N;                                     \
    }
PROBE2(q_add, "1: add %[b],%[a]\n add %[b],%[a]\n add %[b],%[a]\n add %[b],%[a]\n"
              " add %[b],%[a]\n add %[b],%[a]\n add %[b],%[a]\n add %[b],%[a]\n"
              " dec %[n]\n jnz 1b\n")
/* LEA with base+index+disp: latency 3 on p1 (SNB..CLX), 1 on ICL+. x8. */
PROBE2(q_lea3, "1: lea 1(%[a],%[b]),%[a]\n lea 1(%[a],%[b]),%[a]\n"
               " lea 1(%[a],%[b]),%[a]\n lea 1(%[a],%[b]),%[a]\n"
               " lea 1(%[a],%[b]),%[a]\n lea 1(%[a],%[b]),%[a]\n"
               " lea 1(%[a],%[b]),%[a]\n lea 1(%[a],%[b]),%[a]\n"
               " dec %[n]\n jnz 1b\n")
PROBE2(q_imul, "1: imul %[b],%[a]\n imul %[b],%[a]\n imul %[b],%[a]\n imul %[b],%[a]\n"
               " imul %[b],%[a]\n imul %[b],%[a]\n imul %[b],%[a]\n imul %[b],%[a]\n"
               " dec %[n]\n jnz 1b\n")
/* imul (3c) -> instruction under test writes %[a] from %[b] (unrelated). */
#define DEPPROBE(name, insn)                                                 \
    PROBE2(name, "1: imul %[c],%[a]\n " insn "\n imul %[c],%[a]\n " insn "\n"\
                 " imul %[c],%[a]\n " insn "\n imul %[c],%[a]\n " insn "\n"  \
                 " dec %[n]\n jnz 1b\n")
DEPPROBE(d_popcnt, "popcnt %[b],%[a]")
DEPPROBE(d_lzcnt, "lzcnt %[b],%[a]")
DEPPROBE(d_tzcnt, "tzcnt %[b],%[a]")
DEPPROBE(d_bsf, "bsf %[b],%[a]")
DEPPROBE(d_mov, "mov %[b],%[a]")
/* SBB r,r after an independent CMP: result is 0/-1 from CF alone on cores
 * that recognise the idiom (AMD Zen, Bobcat); Intel keeps a read of %[a]. */
DEPPROBE(d_sbbrr, "cmp %[d],%[c]\n sbb %[a],%[a]")

/* Division throughput: independent divides, small operands, TSC per divide. */
static double div_probe(int wide) {
    uint64_t best = ~0ull;
    for (int rep = 0; rep < 5; rep++) {
        uint64_t n = 4000000, x = 1000003, y = 97, q = 0;
        uint64_t t0 = tsc();
        if (wide)
            __asm__ volatile("1: mov %[x],%%rax\n xor %%edx,%%edx\n div %[y]\n add %%rax,%[q]\n dec %[n]\n jnz 1b\n"
                             : [n] "+r"(n), [q] "+r"(q) : [x] "r"(x), [y] "r"(y) : "rax", "rdx", "cc");
        else
            __asm__ volatile("1: mov %[x],%%rax\n xor %%edx,%%edx\n div %k[y]\n add %%rax,%[q]\n dec %[n]\n jnz 1b\n"
                             : [n] "+r"(n), [q] "+r"(q) : [x] "r"(x), [y] "r"(y) : "rax", "rdx", "cc");
        uint64_t t1 = tsc();
        if (t1 - t0 < best) best = t1 - t0;
    }
    return (double)best / 4000000.0;
}

int main(void) {
    double add = q_add() / 8.0; /* TSC ticks per dependent ADD = 1 cycle */
    double k = 1.0 / add;       /* cycles per TSC tick */
    printf("# TSC ticks per core cycle: %.4f (1 / ADD-chain link)\n", add);
    printf("%-34s %8s %s\n", "probe", "cycles", "verdict/expect");
    printf("%-34s %8.2f\n", "add chain (per link)", q_add() / 8.0 * k);
    printf("%-34s %8.2f\n", "imul chain (per link)", q_imul() / 8.0 * k);
    printf("%-34s %8.2f  (1 => ICL+ fast LEA, 3 => p1-only)\n", "lea b+i+d8 chain (per link)", q_lea3() / 8.0 * k);
    const struct { const char *n; double (*f)(void); } dep[] = {
        {"imul->popcnt dest", d_popcnt}, {"imul->lzcnt dest", d_lzcnt},
        {"imul->tzcnt dest", d_tzcnt}, {"imul->bsf dest", d_bsf},
        {"imul->mov dest (control: FREE)", d_mov},
        {"imul->cmp;sbb r,r", d_sbbrr},
    };
    for (unsigned i = 0; i < sizeof dep / sizeof dep[0]; i++) {
        double c = dep[i].f() / 4.0 * k; /* 4 imul+insn pairs per iteration */
        printf("%-34s %8.2f  %s\n", dep[i].n, c, c >= 4.5 ? "DEP (chain through dest)" : "FREE");
    }
    double d64 = div_probe(1) * k, d32 = div_probe(0) * k;
    printf("%-34s %8.2f\n", "div r64 (small operands) rTP", d64);
    printf("%-34s %8.2f\n", "div r32 rTP", d32);
    printf("%-34s %8.2f  (>= 2.0 => bypass_div64 pays)\n", "ratio r64/r32", d64 / d32);
    return 0;
}
