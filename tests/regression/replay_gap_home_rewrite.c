/* Compare-replay home-rewrite regression (kernel 6.18.50 affinity.c).
 *
 * Mirrors get_pcore_mask's structure (the P-core frequency fallback) with
 * kernel dependencies stubbed: two sequential guarded loops over a shared
 * tracked pair (`if (f > max && f != 0) { max = f; max_cpu = cpu; }`),
 * preceded by a sibling-union loop and surrounded by init flags. If-
 * conversion turns each guarded two-assignment update into a Select PAIR
 * sharing one bounds Cmp (multi-select compare-replay). The RA's
 * latch/phi-web coalescing homes the first Select's result and the
 * loop-carried accumulator in ONE register (the latch copy is the value
 * transition), so the first select's cmov rewrites the shared home while
 * the accumulator is still live for the second select's re-emitted
 * compare. The replay's operand read then had no home at all (ICE: "no
 * register, stack slot, Copy, or GlobalAddr definition") or — worse —
 * silently compared the WRONG value.
 *
 * The post-RA gap audit prunes the replay record when a home rewrite sits
 * inside the gap; the Cmp materializes its boolean at its own position and
 * both selects read the carried value BEFORE its home is rewritten.
 * Results are checked differentially against GCC by the driver script; a
 * compile-time ICE is also a failure (the original symptom).
 */
#include <stdio.h>

#define NR_CPUS 32
#define CPU_BITS(n) ((n) < 64 ? (1ull << (n)) : 0ull)
#define cpumask_test_cpu(c, m) (!!((m)[(c) / 64] & (1ull << ((c) % 64))))
#define cpumask_set_cpu(c, m) ((m)[(c) / 64] |= 1ull << ((c) % 64))
#define cpumask_weight_gt1(m) (__builtin_popcountll((m)[0]) > 1)
#define cpumask_empty(m) (!((m)[0] | (m)[1]))

extern unsigned int __cpufreq_quick_get_max(unsigned int cpu);
extern int __hybrid_detected(void);
extern unsigned long long __topo_sibling(int cpu);

static unsigned long long pcore_mask[2];
static int pcore_initialized;

static int get_pcore_mask(unsigned long long *dst, unsigned int ncpus)
{
    unsigned long long local[2] = {0, 0};

    if (!pcore_initialized) {
        int direct = __hybrid_detected();

        /* SMT sibling union fallback (drives register pressure + the
         * early-exit shape of the original). */
        if (!direct) {
            for (unsigned int cpu = 0; cpu < ncpus; cpu++) {
                unsigned long long sib = __topo_sibling((int)cpu);
                if (sib && __builtin_popcountll(sib) > 1)
                    local[0] |= sib;
            }
        }

        /* Frequency-based fallback: THE shape under test. */
        if (cpumask_empty(local)) {
            unsigned int max_freq = 0;
            int max_freq_cpu = -1;

            for (unsigned int cpu = 0; cpu < ncpus; cpu++) {
                unsigned int f = __cpufreq_quick_get_max(cpu);

                if (f > max_freq && f != 0) {
                    max_freq = f;
                    max_freq_cpu = (int)cpu;
                }
            }

            if (max_freq_cpu >= 0 && max_freq > 0) {
                unsigned int threshold = (max_freq / 100U) * 95U;

                for (unsigned int cpu = 0; cpu < ncpus; cpu++) {
                    unsigned int f = __cpufreq_quick_get_max(cpu);

                    if (f >= threshold && f != 0)
                        cpumask_set_cpu((int)cpu, local);
                }
            }
        }

        /* Final fallback: all CPUs. */
        if (cpumask_empty(local)) {
            for (unsigned int cpu = 0; cpu < ncpus && cpu < 64; cpu++)
                local[0] |= 1ull << cpu;
        }

        pcore_mask[0] = local[0];
        pcore_mask[1] = local[1];
        pcore_initialized = 1;
    }

    dst[0] = pcore_mask[0];
    dst[1] = pcore_mask[1];
    return 0;
}

unsigned int __cpufreq_quick_get_max(unsigned int cpu)
{
    return ((cpu * 2654435761u) % 7u) - 1u; /* hits 0, 1..5, UINT_MAX */
}

int __hybrid_detected(void) { return 0; }

unsigned long long __topo_sibling(int cpu)
{
    return cpu % 3 == 0 ? (0x7ull << (cpu % 8)) : 0ull;
}

int main(void)
{
    unsigned long long out[2];
    unsigned int acc = 0;
    for (unsigned int n = 0; n < NR_CPUS; n += 3) {
        pcore_initialized = 0;
        get_pcore_mask(out, n);
        acc += (unsigned int)(out[0] ^ out[1] ^ n);
        printf("n=%u mask=%016llx%016llx acc=%u\n", n, out[1], out[0], acc);
    }
    return acc == 0xDEADBEEFu;
}
