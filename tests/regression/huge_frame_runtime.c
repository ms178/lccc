/* Executed giant-frame addressing.
 *
 * `huge_frame_addressing_o*` prove that frames past 2 GiB *assemble*: their
 * functions are never called, because touching a multi-gigabyte frame costs
 * multi-gigabyte page faults and would destabilise a small runner. That leaves
 * the interesting half unproven -- whether the addresses the backend picks are
 * the right ones at run time.
 *
 * This case closes that gap without needing gigabytes of RAM:
 *
 *   * the frame lives in a *thread* stack, so the whole extent is mapped up
 *     front (`pthread_attr_setstacksize`) and never has to grow through the
 *     main-thread guard gap;
 *   * every access touches only the page it needs, so resident memory stays in
 *     the kilobytes -- the 3 GiB are virtual, not committed;
 *   * the compiler must not probe the frame, which is what
 *     `-fno-stack-clash-protection` (the default) guarantees since the probe
 *     learned to obey its flag. Run this case with
 *     `-fstack-clash-protection` and it deliberately commits the whole frame,
 *     so the case pins the default.
 *
 * The accesses are chosen to hit each addressing decision the backend makes:
 *
 *   %rbp anchor ....... `big[3G-1]`, `big[2G]`, the wide store/load -- near the
 *                      frame register, plain imm32 displacements;
 *   %rsp anchor ....... `big[0]`, `big[G]` -- past 2 GiB from %rbp, but within
 *                      reach of `frame_size + offset` from %rsp;
 *   register .......... the pointer walk, whose address is computed once;
 *   call staging ...... `sink8(...)` takes two far slot loads as its 7th and
 *                      8th arguments, i.e. values read while `%rsp` is displaced
 *                      by the stack-argument pushes;
 *   atomic RMW ........ `atomic_fetch_add_explicit` on a far-addressable object,
 *                      which stages through a push/pop window;
 *   block ops ......... `memset`/`memcpy` across the middle of the object.
 *
 * The printed value is a pure function of the source, so `run_regression.py`
 * compares lccc against gcc-16 and clang-23 directly -- a wrong displacement
 * makes the sum differ, and a wrong *anchor* makes it differ by a power of two
 * that names the mistake.
 *
 * `ulimit -s` is irrelevant here: the frame belongs to a thread whose stack is
 * sized explicitly. A runner that cannot map 4 GiB of address space fails the
 * `pthread_create` below and reports exit code 4 rather than a silent
 * difference.
 */

#define _GNU_SOURCE 1

#include <pthread.h>
#include <stdatomic.h>
#include <stdio.h>
#include <string.h>

#define HF_G ((unsigned long)1 << 30)

/* Eight arguments: the last two are pushed, so their evaluation happens with
 * `%rsp` moved. `noinline` keeps the call site real at every level. */
__attribute__((noinline)) static unsigned long hf_sink8(unsigned long a, unsigned long b,
                                                        unsigned long c, unsigned long d,
                                                        unsigned long e, unsigned long f,
                                                        unsigned long g, unsigned long h)
{
    return a + 2 * b + 3 * c + 4 * d + 5 * e + 6 * f + 7 * g + 8 * h;
}

static _Atomic unsigned long hf_shared;

__attribute__((noinline)) static unsigned long hf_frame_body(void)
{
    /* `_Alignas` so the 8-byte accesses below are aligned: an unaligned
     * access through a `volatile unsigned long *` is undefined behaviour, and
     * the oracles legitimately disagree about what it produces. */
    _Alignas(16) volatile unsigned char big[3 * HF_G];
    unsigned char tmp[16];
    unsigned long sum = 0;
    int i;

    /* Both anchors and the ends of the object. */
    big[0] = 11;
    big[HF_G] = 22;
    big[2 * HF_G] = 33;
    big[3 * HF_G - 1] = 44;
    sum += big[0] + big[HF_G] + big[2 * HF_G] + big[3 * HF_G - 1];

    /* An 8-byte store and load whose displacement is far from %rbp but inside
     * the imm32 window from %rsp. */
    {
        volatile unsigned long *q = (volatile unsigned long *)&big[2 * HF_G];
        q[0] = 0x0123456789abcdefUL;
        sum += (unsigned long)q[0];
    }

    /* Register addressing, one stride per anchor band. */
    {
        volatile unsigned char *p = big;
        for (i = 0; i < 8; i++) {
            p[(unsigned long)i * (HF_G / 8)] = (unsigned char)(i * 3 + 1);
        }
        for (i = 0; i < 8; i++) {
            sum += p[(unsigned long)i * (HF_G / 8)];
        }
    }

    /* Stack arguments staged while %rsp is displaced. */
    sum += hf_sink8(1, 2, 3, 4, 5, 6, big[HF_G], big[0]);

    /* An atomic read-modify-write through a far address. */
    atomic_store_explicit(&hf_shared, 100, memory_order_relaxed);
    sum += atomic_fetch_add_explicit(&hf_shared, 5, memory_order_seq_cst);
    sum += atomic_load_explicit(&hf_shared, memory_order_relaxed);

    /* Block operations across the middle band of the object. */
    __builtin_memset((void *)&big[HF_G / 2], 0x5a, 16);
    __builtin_memcpy(tmp, (void *)&big[HF_G / 2], 16);
    sum += tmp[0] + tmp[15] + big[HF_G / 2];

    return sum;
}

static void *hf_thread(void *arg)
{
    (void)arg;
    return (void *)hf_frame_body();
}

int main(void)
{
    pthread_attr_t attr;
    pthread_t thread;
    void *result = NULL;

    if (pthread_attr_init(&attr) != 0) {
        return 2;
    }
    /* 4 GiB covers the 3 GiB frame plus the pthread guard and the runtime's
     * own frames. The mapping is lazy: only the pages the body touches are
     * ever committed. */
    if (pthread_attr_setstacksize(&attr, 4 * HF_G) != 0) {
        return 3;
    }
    if (pthread_create(&thread, &attr, hf_thread, NULL) != 0) {
        return 4;
    }
    if (pthread_join(thread, &result) != 0) {
        return 5;
    }
    printf("%lu\n", (unsigned long)result);
    return 0;
}
