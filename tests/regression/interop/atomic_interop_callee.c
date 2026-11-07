#include "atomic_interop.h"

u32 interop_add32(u32 *p, u32 value)
{
#ifdef ATOMIC_INTEROP_MUTANT
    /* Negative control: the caller must observe this wrong update. */
    value += 1;
#endif
    return __atomic_fetch_add(p, value, __ATOMIC_SEQ_CST);
}

int interop_cas32(u32 *p, u32 *expected, u32 desired)
{
    return __atomic_compare_exchange_n(p, expected, desired, 0,
                                       __ATOMIC_ACQ_REL, __ATOMIC_ACQUIRE);
}

u64 interop_xchg64(u64 *p, u64 value)
{
    return __atomic_exchange_n(p, value, __ATOMIC_SEQ_CST);
}

#if defined(__aarch64__)
u128 interop_add128(u128 *p, u128 value)
{
    return __atomic_fetch_add(p, value, __ATOMIC_SEQ_CST);
}

int interop_cas128(u128 *p, u128 *expected, u128 desired)
{
    return __atomic_compare_exchange_n(p, expected, desired, 0,
                                       __ATOMIC_ACQ_REL, __ATOMIC_ACQUIRE);
}

u128 interop_xchg128(u128 *p, u128 value)
{
    return __atomic_exchange_n(p, value, __ATOMIC_SEQ_CST);
}
#endif
