/* `&obj` carries the object's named address space (__seg_gs / kernel __percpu).
 *
 * WHY THIS TEST EXISTS
 * --------------------
 * A Linux per-CPU object is declared in the GS segment:
 *
 *     static DEFINE_PER_CPU(struct tasklet_head, tasklet_vec);
 *     -> static __seg_gs __attribute__((section(".data..percpu")))
 *            __typeof__(struct tasklet_head) tasklet_vec;
 *
 * and handed to a function whose parameter is __percpu-qualified:
 *
 *     __tasklet_schedule_common(t, &tasklet_vec, TASKLET_SOFTIRQ);
 *     static void __tasklet_schedule_common(struct tasklet_struct *t,
 *                                           struct tasklet_head __percpu *headp, ...)
 *
 * `&tasklet_vec` must therefore have type `__seg_gs struct tasklet_head *`, or
 * the call is rejected with "have 'struct tasklet_head *' but expected
 * '__seg_gs struct tasklet_head *'" and kernel/softirq.c does not compile
 * (the diagnostic pointed at kernel/softirq.c:671/677 in the 6.18.55 build).
 *
 * The same rule reaches through members and subscripts, and it must NOT be
 * over-applied: GCC reads `__seg_gs` as a type QUALIFIER, so in a pointer
 * declaration the space belongs to the POINTEE while the object keeps default
 * storage. Measured with gcc 14.2 for `static __seg_gs T *p;`:
 * `T **b = &p;` is REJECTED ("&p" has type `__seg_gs T **`) and
 * `T __seg_gs **c = &p;` is ACCEPTED. The same shape occurs in
 * include/linux/percpu-refcount.h: `__ref_is_percpu(ref, &percpu_count)` where
 * percpu_count is `ulong __percpu *`; applying the declaration space to the
 * object unconditionally double-qualifies it there and breaks asm-offsets.s.
 *
 * Nothing here dereferences a segment pointer: %gs is not set up in this test
 * process, so only the types and the addresses are exercised.
 */

extern int printf(const char *, ...);

#define __percpu __seg_gs

typedef unsigned long ulong;

struct tasklet_head {
    void *head;
    void *tail;
};

/* File-scope per-CPU object, exactly the DEFINE_PER_CPU expansion shape. */
static __seg_gs __attribute__((section(".data..percpu")))
__typeof__(struct tasklet_head) tasklet_vec;
static __seg_gs __typeof__(struct tasklet_head) tasklet_hi_vec;

/* A default-space object of the same type, for contrast. */
static struct tasklet_head plain_vec;

/* The kernel's parameter shape: an __percpu-qualified pointer. Addresses are
 * kept as integers: a cast from a segment pointer to a generic `void *` is a
 * warning in GCC ("disjoint address space"), and the property under test is
 * the type of the argument, not a comparison of unlike pointers. */
static ulong seen[4];
static int nseen;

static void schedule_common(void *t, struct tasklet_head __percpu *headp)
{
    if (nseen < 4)
        seen[nseen++] = (ulong)headp;
    (void)t;
}

/* `&percpu_obj->member` must also be accepted (member lives in the segment). */
static ulong tail_of(struct tasklet_head __percpu *headp)
{
    return (ulong)&headp->tail;
}

/* Pointer declarator: the space qualifies the POINTEE, so the object `p`
 * itself has default storage and `&p` reaches a `__percpu **` parameter. */
static int is_percpu_ref(ulong __percpu **percpu_countp)
{
    return *percpu_countp == 0;
}

struct percpu_ref {
    ulong __percpu *percpu_count;
};

int main(void)
{
    int rc = 0;
    ulong __percpu *local_count = 0;
    struct percpu_ref ref;
    ulong tail;

    ref.percpu_count = 0;

    /* 1. &object of a per-CPU variable, straight into the __percpu parameter. */
    schedule_common((void *)0, &tasklet_vec);
    schedule_common((void *)0, &tasklet_hi_vec);
    if (nseen != 2)
        rc = 1;

    /* 2. The address survives a member access on a per-CPU object. */
    tail = tail_of(&tasklet_vec);
    if (tail != (ulong)&tasklet_vec.tail)
        rc = 2;

    /* 3. &object really is the object's address, and distinct per object. */
    if (seen[0] == seen[1])
        rc = 3;
    if (seen[0] != (ulong)&tasklet_vec)
        rc = 4;
    if ((ulong)&tasklet_vec == (ulong)&plain_vec)
        rc = 5;

    /* 4. Pointer-declarator absorption (no double __seg_gs). */
    if (!is_percpu_ref(&local_count))
        rc = 6;
    if (!is_percpu_ref(&ref.percpu_count))
        rc = 7;

    printf("seg-gs: nseen=%d distinct=%d tail_ok=%d refs_ok=%d rc=%d\n",
           nseen,
           (int)(seen[0] != seen[1]),
           (int)(tail == (ulong)&tasklet_vec.tail),
           (int)(is_percpu_ref(&local_count) && is_percpu_ref(&ref.percpu_count)),
           rc);
    return rc;
}
