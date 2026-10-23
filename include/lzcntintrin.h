/* LCCC bundled lzcntintrin.h - LZCNT intrinsics (GCC/Clang names).
 *
 * LZCNT is defined at zero (the operand width). The guarded form folds to
 * the zero-defined count, which -mlzcnt emits as one `lzcnt`. */
#ifndef _LZCNTINTRIN_H_INCLUDED
#define _LZCNTINTRIN_H_INCLUDED

static __inline__ unsigned short __attribute__((__always_inline__))
__lzcnt16(unsigned short __x)
{
    return __x ? (unsigned short)(__builtin_clz(__x) - 16) : 16;
}

static __inline__ unsigned int __attribute__((__always_inline__))
__lzcnt32(unsigned int __x)
{
    return __x ? (unsigned int)__builtin_clz(__x) : 32;
}

static __inline__ unsigned int __attribute__((__always_inline__))
_lzcnt_u32(unsigned int __x)
{
    return __lzcnt32(__x);
}

#ifdef __x86_64__
static __inline__ unsigned long long __attribute__((__always_inline__))
__lzcnt64(unsigned long long __x)
{
    return __x ? (unsigned long long)__builtin_clzll(__x) : 64;
}

static __inline__ unsigned long long __attribute__((__always_inline__))
_lzcnt_u64(unsigned long long __x)
{
    return __lzcnt64(__x);
}
#endif /* __x86_64__ */

#endif /* _LZCNTINTRIN_H_INCLUDED */
