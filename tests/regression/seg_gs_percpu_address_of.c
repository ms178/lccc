/* `&obj` must keep the object's named address space (__seg_gs == kernel __percpu).
 *
 * WHY THIS TEST EXISTS
 * --------------------
 * Linux declares per-CPU objects in the GS segment:
 *
 *     DEFINE_PER_CPU(struct tasklet_head, tasklet_vec)
 * --> static __seg_gs __attribute__((section(".data..percpu")))
 *         __typeof__(struct tasklet_head) tasklet_vec;
 *
 * and passes their address to a `__percpu`-qualified parameter:
 *
 *     __tasklet_schedule_common(t, &tasklet_vec, TASKLET_SOFTIRQ);
 *     static void __tasklet_schedule_common(struct tasklet_struct *t,
 *                                           struct tasklet_head __percpu *headp, ...)
 *
 * before the fix lccc typed `&tasklet_vec` as a plain `struct tasklet_head *`
 * and rejected the call:
 *
 *     error: incompatible pointer type for argument 2 to
 *     '__tasklet_schedule_common': have 'struct tasklet_head *' but
 *     expected '__seg_gs struct tasklet_head *'   (kernel/softirq.c, 2 errors)
 *
 * The space belongs to the OBJECT's storage, not to its value, so it survives
 * `&` — and it must NOT be double-applied through a pointer declarator, where
 * GCC reads `__seg_gs` as a qualifier on the type name (measured with gcc 14.2,
 * `static __seg_gs T *p;`): `T **b = &p;` is REJECTED, while
 * `T __seg_gs **c = &p;` is ACCEPTED. Applying the object space there too
 * produced `have '__seg_gs __seg_gs unsigned long **'` and killed
 * arch/x86/kernel/asm-offsets.s via include/linux/percpu-refcount.h.
 *
 * Everything here is address-taking and type-checking only: the test never
 * dereferences a segment pointer, so it runs to completion with GS unset.
 */
#include <stdio.h>

struct tasklet_head {
    void *head;
    void *tail;
};

/* The kernel's DEFINE_PER_CPU expansion shape, verbatim. */
static __seg_gs __attribute__((section(".data..percpu")))
__typeof__(struct tasklet_head) tasklet_vec;
static __seg_gs __attribute__((section(".data..percpu")))
__typeof__(struct tasklet_head) tasklet_hi_vec;

/* A pointer object in the segment: the space qualifies the POINTEE here. */
static __seg_gs struct tasklet_head *tasklet_ptr;

/* Parameters of the two shapes the kernel uses. */
static int take_percpu(struct tasklet_head __seg_gs *headp)
{
    return headp != 0;
}
static int take_percpu_ptrptr(struct tasklet_head __seg_gs **pp)
{
    return pp != 0;
}

int main(void)
{
    int fails = 0;
    const char *why = "ok";

    /* &percpu_object into a __percpu-qualified parameter. */
    if (!take_percpu(&tasklet_vec)) { fails++; why = "&tasklet_vec rejected or null"; }
    if (!take_percpu(&tasklet_hi_vec)) { fails++; why = "&tasklet_hi_vec rejected or null"; }
    /* Through a member access, and through the __typeof__ form. */
    if (!take_percpu(&tasklet_vec.tail ? &tasklet_hi_vec : &tasklet_vec)) {
        fails++;
        why = "member-derived segment address rejected";
    }
    /* Distinct objects stay distinct after the space is attached. */
    if (&tasklet_vec == &tasklet_hi_vec) { fails++; why = "distinct per-CPU objects compared equal"; }

    /* Pointer declarator: &p is a __seg_gs T ** (NOT double-qualified). */
    if (!take_percpu_ptrptr(&tasklet_ptr)) { fails++; why = "&tasklet_ptr rejected"; }

    printf("seg_gs: fails=%d (%s)\n", fails, why);
    return fails != 0;
}
