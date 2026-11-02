/* LCCC bundled popcntintrin.h - POPCNT intrinsics (GCC/Clang names).
 *
 * Portable, like the BMI/LZCNT headers: the population count is
 * `__builtin_popcount`, which a function granted POPCNT (`-mpopcnt`, a
 * v2+ `-march=`, `target("popcnt")`) emits as one `popcnt` and any other
 * function as the branch-free bit-count sequence. GCC instead rejects the
 * call outside POPCNT code; accepting it never yields an instruction the
 * function's target lacks. */
#ifndef _POPCNTINTRIN_H_INCLUDED
#define _POPCNTINTRIN_H_INCLUDED

static __inline__ int __attribute__((__always_inline__))
_mm_popcnt_u32(unsigned int __x)
{
    return __builtin_popcount(__x);
}

#ifdef __x86_64__
static __inline__ long long __attribute__((__always_inline__))
_mm_popcnt_u64(unsigned long long __x)
{
    return __builtin_popcountll(__x);
}
#endif

#endif /* _POPCNTINTRIN_H_INCLUDED */
