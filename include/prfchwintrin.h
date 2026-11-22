/* CCC compiler bundled prfchwintrin.h - PREFETCHW */
#ifndef _PRFCHWINTRIN_H_INCLUDED
#define _PRFCHWINTRIN_H_INCLUDED

/* Prefetch the cache line of __P for a write: `prefetchw` where the target
 * has PRFCHW, the instruction of locality 3 (`prefetcht0`) elsewhere, as
 * GCC 16.2 selects for the same call. Not behind `__PRFCHW__`, as GCC's
 * is not: the name is callable for every target. */
static __inline__ void __attribute__((__always_inline__))
_m_prefetchw(volatile const void *__P)
{
    __builtin_prefetch((const void *)__P, 1, 3 /* _MM_HINT_T0 */);
}

#endif /* _PRFCHWINTRIN_H_INCLUDED */
