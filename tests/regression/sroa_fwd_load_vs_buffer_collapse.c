/* aggregate_sroa: Transform 1 (load forwarding) vs Transform 2 (copy-buffer
 * collapse) interaction on by-value struct parameter bridges.
 *
 * Shape (linux-6.18 arch/x86/mm/pgtable.c pte_mkwrite, miscompiled by
 * lccc main 8ad39bfe): an out-of-line function takes an 8-byte struct BY
 * VALUE. The IR inliner materializes the argument as Memcpy bridges
 * (callee home <- caller home), and a second-level inlined helper creates a
 * double-buffer chain (L2 home <- L1 home <- param home). aggregate_sroa's
 * Transform 1 forwards the struct load to the L1 buffer, while Transform 2
 * - still scanning the pre-plan stream where the L1 buffer looks
 * memcpy-only - deletes the L1 buffer's initializing Memcpy. The forwarded
 * load then reads an uninitialized stack slot: pte_mkwrite returned
 * leftover stack garbage as the PTE (every execve failed with -E2BIG,
 * "No working init" panic).
 *
 * The struct must flow through THREE inlined call levels (mkwrite -> novma
 * -> setf2 -> setf) so the forwarded load lands on the MIDDLE buffer; two
 * levels let a later pass re-forward it to the param home and hide the bug.
 * Helpers are `static inline` (noinline helpers are never inlined here and
 * pass the struct through the real ABI). The branch mask is the constant
 * zero (VM_SHADOW_STACK with the config disabled) so everything folds into
 * one basic block - aggregate_sroa only forwards loads within one block.
 */
#include <stdio.h>

typedef struct { unsigned long v; } pte_t;
struct vma { unsigned long flags; };

static inline pte_t setf(pte_t p, unsigned long m)
{
    /* functional style, like pte_set_flags: the by-value param home is only
     * READ; the result lands in a fresh local. An in-place `p.v |= m; return
     * p;` would store through the param home and hide the bug. */
    pte_t r;
    r.v = p.v | m;
    return r;
}

static inline pte_t setf2(pte_t p, unsigned long m)
{
    /* third level: setf's home is initialized from novma's home, which is
     * itself initialized from the param home - the double-buffer chain that
     * strands the forwarded load on the middle buffer. */
    return setf(p, m);
}

static inline pte_t novma(pte_t p)
{
    return setf2(p, 2UL); /* second-level bridge: L2 home <- L1 home */
}

static inline pte_t clrsd(pte_t p)
{
    /* clear_saveddirty analog: param home is read-only, fresh result */
    pte_t r;
    r.v = (p.v & 2UL) ? (p.v & ~(1UL << 58)) : p.v;
    return r;
}

/* mirror of the kernel's pte_mkwrite: struct by value + pointer, early
 * branch on the pointer's field, struct flows through the helper chain */
pte_t mkwrite(pte_t p, struct vma *v)
{
    /* constant-zero mask (VM_SHADOW_STACK with the config disabled): the
     * branch folds away and everything lands in ONE block - aggregate_sroa
     * only forwards loads dominated within the same basic block. */
    if (v->flags & 0UL)
        return p;
    /* second local kept live across both calls: prevents the backend from
     * coalescing the (uninitialized) stranded buffer's slot with the param
     * home's slot, which would silently hide the bug at runtime. */
    pte_t q;
    q.v = p.v ^ 0xdeadbeefUL;
    p = novma(p);
    p = clrsd(p); /* second by-value call chained through the param home */
    p.v ^= q.v;
    return p;
}

/* leave recognizable poison on the stack for a callee to pick up */
__attribute__((noinline)) static void stack_poison(void)
{
    unsigned long buf[32];
    for (int i = 0; i < 32; i++)
        buf[i] = 0xffff888000000000UL + i;
    __asm__ volatile("" ::"r"(buf) : "memory");
}

int main(void)
{
    struct vma v = { .flags = 0 };
    int fail = 0;

    for (int i = 0; i < 4; i++) {
        pte_t in = { .v = 0x8000000001dff867UL };
        pte_t out;
        /* q = in ^ 0xdeadbeef, then result ^= q: expected value is
         * ((in | 2) with bit 58 cleared) ^ in ^ 0xdeadbeef */
        unsigned long expect = ((0x8000000001dff867UL | 2UL) & ~(1UL << 58))
                               ^ 0x8000000001dff867UL ^ 0xdeadbeefUL;
        stack_poison();
        out = mkwrite(in, &v);
        if (out.v != expect) {
            printf("FAIL mkwrite pass %d: got %lx want %lx\n", i, out.v, expect);
            fail++;
        }
    }

    if (!fail)
        printf("PASS\n");
    return fail ? 1 : 0;
}
