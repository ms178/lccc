/* LCCC bundled ia32intrin.h - scalar ia32 helpers (GCC layout: reached
 * from both immintrin.h and x86intrin.h). */
#ifndef _IA32INTRIN_H_INCLUDED
#define _IA32INTRIN_H_INCLUDED

/* rdtsc - Read Time-Stamp Counter */
static __inline__ unsigned long long
__attribute__((__always_inline__))
__rdtsc(void)
{
    unsigned int __lo, __hi;
    __asm__ __volatile__("rdtsc" : "=a"(__lo), "=d"(__hi));
    return ((unsigned long long)__hi << 32) | __lo;
}

/* rdtscp - Read Time-Stamp Counter and Processor ID */
static __inline__ unsigned long long
__attribute__((__always_inline__))
__rdtscp(unsigned int *__aux)
{
    unsigned int __lo, __hi;
    __asm__ __volatile__("rdtscp" : "=a"(__lo), "=d"(__hi), "=c"(*__aux));
    return ((unsigned long long)__hi << 32) | __lo;
}

/* Compatibility aliases */
#define _rdtsc()      __rdtsc()
#define _rdtscp(a)    __rdtscp(a)

/* Bit-scan intrinsics: undefined for zero, like BSF/BSR and GCC. */
static __inline__ int
__attribute__((__always_inline__))
__bsfd(int __a)
{
    return __builtin_ctz((unsigned int)__a);
}

static __inline__ int
__attribute__((__always_inline__))
__bsrd(int __a)
{
    return 31 ^ __builtin_clz((unsigned int)__a);
}

/* Byte-swap intrinsics */
static __inline__ int
__attribute__((__always_inline__))
__bswapd(int __a)
{
    return __builtin_bswap32(__a);
}

#ifdef __x86_64__
static __inline__ long long
__attribute__((__always_inline__))
__bswapq(long long __a)
{
    return __builtin_bswap64(__a);
}

static __inline__ int
__attribute__((__always_inline__))
__bsfq(long long __a)
{
    return __builtin_ctzll((unsigned long long)__a);
}

static __inline__ int
__attribute__((__always_inline__))
__bsrq(long long __a)
{
    return 63 ^ __builtin_clzll((unsigned long long)__a);
}
#endif /* __x86_64__ */

/* Pause instruction - hint for spin-wait loops */
static __inline__ void
__attribute__((__always_inline__))
__pause(void)
{
    __asm__ __volatile__("pause");
}

/* Rotation intrinsics. The complementary shift uses (-n & (w-1)) so a
 * rotate by a multiple of the width never shifts by the full width (C UB). */
static __inline__ unsigned char
__attribute__((__always_inline__))
__rolb(unsigned char __a, int __n)
{
    return (unsigned char)((__a << (__n & 7)) | (__a >> (8 - (__n & 7))));
}

static __inline__ unsigned short
__attribute__((__always_inline__))
__rolw(unsigned short __a, int __n)
{
    return (unsigned short)((__a << (__n & 15)) | (__a >> (16 - (__n & 15))));
}

static __inline__ unsigned int
__attribute__((__always_inline__))
__rold(unsigned int __a, int __n)
{
    return (__a << (__n & 31)) | (__a >> (-__n & 31));
}

static __inline__ unsigned char
__attribute__((__always_inline__))
__rorb(unsigned char __a, int __n)
{
    return (unsigned char)((__a >> (__n & 7)) | (__a << (8 - (__n & 7))));
}

static __inline__ unsigned short
__attribute__((__always_inline__))
__rorw(unsigned short __a, int __n)
{
    return (unsigned short)((__a >> (__n & 15)) | (__a << (16 - (__n & 15))));
}

static __inline__ unsigned int
__attribute__((__always_inline__))
__rord(unsigned int __a, int __n)
{
    return (__a >> (__n & 31)) | (__a << (-__n & 31));
}

#ifdef __x86_64__
static __inline__ unsigned long long
__attribute__((__always_inline__))
__rolq(unsigned long long __a, int __n)
{
    return (__a << (__n & 63)) | (__a >> (-__n & 63));
}

static __inline__ unsigned long long
__attribute__((__always_inline__))
__rorq(unsigned long long __a, int __n)
{
    return (__a >> (__n & 63)) | (__a << (-__n & 63));
}
#endif /* __x86_64__ */

/* Population count. */
static __inline__ int __attribute__((__always_inline__))
__popcntd(unsigned int __a)
{
    return __builtin_popcount(__a);
}

#ifdef __x86_64__
static __inline__ long long __attribute__((__always_inline__))
__popcntq(unsigned long long __a)
{
    return __builtin_popcountll(__a);
}
#endif

/* GCC/ICC ia32 aliases. */
#define _bit_scan_forward(a) __bsfd(a)
#define _bit_scan_reverse(a) __bsrd(a)
#define _bswap(a) __bswapd(a)
#define _popcnt32(a) __popcntd(a)
#define _rotl(a, b) __rold((a), (b))
#define _rotr(a, b) __rord((a), (b))
#define _rotwl(a, b) __rolw((a), (b))
#define _rotwr(a, b) __rorw((a), (b))
#ifdef __x86_64__
#define _bswap64(a) __bswapq(a)
#define _popcnt64(a) __popcntq(a)
#define _lrotl(a, b) __rolq((a), (b))
#define _lrotr(a, b) __rorq((a), (b))
#else
#define _lrotl(a, b) __rold((a), (b))
#define _lrotr(a, b) __rord((a), (b))
#endif

#endif /* _IA32INTRIN_H_INCLUDED */
