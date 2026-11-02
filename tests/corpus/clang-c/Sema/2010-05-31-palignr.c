// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
# 1 "Sema/2010-05-31-palignr.c"
# 1 "<built-in>" 1
# 1 "<built-in>" 3
# 412 "<built-in>" 3
# 1 "<command line>" 1
# 1 "<built-in>" 2
# 1 "Sema/2010-05-31-palignr.c" 2


# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/tmmintrin.h" 1
# 17 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/tmmintrin.h"
# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/pmmintrin.h" 1
# 17 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/pmmintrin.h"
# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h" 1
# 17 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h" 1
# 17 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h" 1
# 17 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
typedef long long __m64 __attribute__((__vector_size__(8), __aligned__(8)));

typedef long long __v1di __attribute__((__vector_size__(8)));
typedef int __v2si __attribute__((__vector_size__(8)));
typedef short __v4hi __attribute__((__vector_size__(8)));
typedef char __v8qi __attribute__((__vector_size__(8)));


typedef unsigned long long __v1du __attribute__ ((__vector_size__ (8)));
typedef unsigned int __v2su __attribute__ ((__vector_size__ (8)));
typedef unsigned short __v4hu __attribute__((__vector_size__(8)));
typedef unsigned char __v8qu __attribute__((__vector_size__(8)));



typedef signed char __v8qs __attribute__((__vector_size__(8)));


typedef long long __m128i __attribute__((__vector_size__(16), __aligned__(16)));
typedef long long __v2di __attribute__ ((__vector_size__ (16)));
typedef int __v4si __attribute__((__vector_size__(16)));
typedef short __v8hi __attribute__((__vector_size__(16)));
typedef char __v16qi __attribute__((__vector_size__(16)));
# 65 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ void
    __attribute__((__always_inline__, __nodebug__, __target__("mmx")))
    _mm_empty(void) {
  __builtin_ia32_emms();
}
# 82 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cvtsi32_si64(int __i) {
  return __extension__(__m64)(__v2si){__i, 0};
}
# 97 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ int __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cvtsi64_si32(__m64 __m) {
  return ((__v2si)__m)[0];
}
# 111 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cvtsi64_m64(long long __i) {
  return __extension__(__m64)(__v1di){__i};
}
# 125 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ long long __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cvtm64_si64(__m64 __m) {
  return ((__v1di)__m)[0];
}
# 148 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_packs_pi16(__m64 __m1,
                                                               __m64 __m2) {
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_packsswb128( (__v8hi)__builtin_shufflevector(__m1, __m2, 0, 1), (__v8hi){})), __extension__(__v2di){}, 0);

}
# 173 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_packs_pi32(__m64 __m1,
                                                               __m64 __m2) {
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_packssdw128( (__v4si)__builtin_shufflevector(__m1, __m2, 0, 1), (__v4si){})), __extension__(__v2di){}, 0);

}
# 198 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_packs_pu16(__m64 __m1,
                                                               __m64 __m2) {
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_packuswb128( (__v8hi)__builtin_shufflevector(__m1, __m2, 0, 1), (__v8hi){})), __extension__(__v2di){}, 0);

}
# 225 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_unpackhi_pi8(__m64 __m1,
                                                                 __m64 __m2) {
  return (__m64)__builtin_shufflevector((__v8qi)__m1, (__v8qi)__m2, 4, 12, 5,
                                        13, 6, 14, 7, 15);
}
# 248 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_unpackhi_pi16(__m64 __m1,
                                                                  __m64 __m2) {
  return (__m64)__builtin_shufflevector((__v4hi)__m1, (__v4hi)__m2, 2, 6, 3, 7);
}
# 268 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_unpackhi_pi32(__m64 __m1,
                                                                  __m64 __m2) {
  return (__m64)__builtin_shufflevector((__v2si)__m1, (__v2si)__m2, 1, 3);
}
# 294 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_unpacklo_pi8(__m64 __m1,
                                                                 __m64 __m2) {
  return (__m64)__builtin_shufflevector((__v8qi)__m1, (__v8qi)__m2, 0, 8, 1, 9,
                                        2, 10, 3, 11);
}
# 317 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_unpacklo_pi16(__m64 __m1,
                                                                  __m64 __m2) {
  return (__m64)__builtin_shufflevector((__v4hi)__m1, (__v4hi)__m2, 0, 4, 1, 5);
}
# 337 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_unpacklo_pi32(__m64 __m1,
                                                                  __m64 __m2) {
  return (__m64)__builtin_shufflevector((__v2si)__m1, (__v2si)__m2, 0, 2);
}
# 357 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_add_pi8(__m64 __m1,
                                                            __m64 __m2) {
  return (__m64)(((__v8qu)__m1) + ((__v8qu)__m2));
}
# 377 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_add_pi16(__m64 __m1,
                                                             __m64 __m2) {
  return (__m64)(((__v4hu)__m1) + ((__v4hu)__m2));
}
# 397 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_add_pi32(__m64 __m1,
                                                             __m64 __m2) {
  return (__m64)(((__v2su)__m1) + ((__v2su)__m2));
}
# 420 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_adds_pi8(__m64 __m1,
                                                             __m64 __m2) {
  return (__m64)__builtin_elementwise_add_sat((__v8qs)__m1, (__v8qs)__m2);
}
# 443 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_adds_pi16(__m64 __m1,
                                                              __m64 __m2) {
  return (__m64)__builtin_elementwise_add_sat((__v4hi)__m1, (__v4hi)__m2);
}
# 465 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_adds_pu8(__m64 __m1,
                                                             __m64 __m2) {
  return (__m64)__builtin_elementwise_add_sat((__v8qu)__m1, (__v8qu)__m2);
}
# 487 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_adds_pu16(__m64 __m1,
                                                              __m64 __m2) {
  return (__m64)__builtin_elementwise_add_sat((__v4hu)__m1, (__v4hu)__m2);
}
# 507 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_sub_pi8(__m64 __m1,
                                                            __m64 __m2) {
  return (__m64)(((__v8qu)__m1) - ((__v8qu)__m2));
}
# 527 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_sub_pi16(__m64 __m1,
                                                             __m64 __m2) {
  return (__m64)(((__v4hu)__m1) - ((__v4hu)__m2));
}
# 547 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_sub_pi32(__m64 __m1,
                                                             __m64 __m2) {
  return (__m64)(((__v2su)__m1) - ((__v2su)__m2));
}
# 570 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_subs_pi8(__m64 __m1,
                                                             __m64 __m2) {
  return (__m64)__builtin_elementwise_sub_sat((__v8qs)__m1, (__v8qs)__m2);
}
# 593 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_subs_pi16(__m64 __m1,
                                                              __m64 __m2) {
  return (__m64)__builtin_elementwise_sub_sat((__v4hi)__m1, (__v4hi)__m2);
}
# 616 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_subs_pu8(__m64 __m1,
                                                             __m64 __m2) {
  return (__m64)__builtin_elementwise_sub_sat((__v8qu)__m1, (__v8qu)__m2);
}
# 639 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_subs_pu16(__m64 __m1,
                                                              __m64 __m2) {
  return (__m64)__builtin_elementwise_sub_sat((__v4hu)__m1, (__v4hu)__m2);
}
# 665 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_madd_pi16(__m64 __m1,
                                                              __m64 __m2) {
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_pmaddwd128((__v8hi)(__m128i) __builtin_shufflevector((__v2si)(__m1), __extension__(__v2si){}, 0, 1, 2, 3), (__v8hi)(__m128i) __builtin_shufflevector((__v2si)(__m2), __extension__(__v2si){}, 0, 1, 2, 3))), __extension__(__v2di){}, 0);

}
# 686 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_mulhi_pi16(__m64 __m1,
                                                               __m64 __m2) {
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_pmulhw128((__v8hi)(__m128i) __builtin_shufflevector((__v2si)(__m1), __extension__(__v2si){}, 0, 1, 2, 3), (__v8hi)(__m128i) __builtin_shufflevector((__v2si)(__m2), __extension__(__v2si){}, 0, 1, 2, 3))), __extension__(__v2di){}, 0);

}
# 707 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_mullo_pi16(__m64 __m1,
                                                               __m64 __m2) {
  return (__m64)(((__v4hu)__m1) * ((__v4hu)__m2));
}
# 729 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_sll_pi16(__m64 __m, __m64 __count)
{
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_psllw128((__v8hi)(__m128i) __builtin_shufflevector((__v2si)(__m), __extension__(__v2si){}, 0, 1, 2, 3), (__v8hi)(__m128i) __builtin_shufflevector((__v2si)(__count), __extension__(__v2si){}, 0, 1, 2, 3))), __extension__(__v2di){}, 0);

}
# 752 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_slli_pi16(__m64 __m,
                                                              int __count) {
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_psllwi128((__v8hi)(__m128i) __builtin_shufflevector((__v2si)(__m), __extension__(__v2si){}, 0, 1, 2, 3), __count)), __extension__(__v2di){}, 0);
}
# 774 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_sll_pi32(__m64 __m, __m64 __count)
{
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_pslld128((__v4si)(__m128i) __builtin_shufflevector((__v2si)(__m), __extension__(__v2si){}, 0, 1, 2, 3), (__v4si)(__m128i) __builtin_shufflevector((__v2si)(__count), __extension__(__v2si){}, 0, 1, 2, 3))), __extension__(__v2di){}, 0);

}
# 797 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_slli_pi32(__m64 __m,
                                                              int __count) {
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_pslldi128((__v4si)(__m128i) __builtin_shufflevector((__v2si)(__m), __extension__(__v2si){}, 0, 1, 2, 3), __count)), __extension__(__v2di){}, 0);
}
# 816 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_sll_si64(__m64 __m, __m64 __count)
{
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_psllq128((__v2di)(__m128i) __builtin_shufflevector((__v2si)(__m), __extension__(__v2si){}, 0, 1, 2, 3), (__v2di)(__m128i) __builtin_shufflevector((__v2si)(__count), __extension__(__v2si){}, 0, 1, 2, 3))), __extension__(__v2di){}, 0);

}
# 837 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_slli_si64(__m64 __m,
                                                              int __count) {
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_psllqi128((__v2di)(__m128i) __builtin_shufflevector((__v2si)(__m), __extension__(__v2si){}, 0, 1, 2, 3), __count)), __extension__(__v2di){}, 0);
}
# 860 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_sra_pi16(__m64 __m, __m64 __count)
{
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_psraw128((__v8hi)(__m128i) __builtin_shufflevector((__v2si)(__m), __extension__(__v2si){}, 0, 1, 2, 3), (__v8hi)(__m128i) __builtin_shufflevector((__v2si)(__count), __extension__(__v2si){}, 0, 1, 2, 3))), __extension__(__v2di){}, 0);

}
# 884 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_srai_pi16(__m64 __m,
                                                              int __count) {
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_psrawi128((__v8hi)(__m128i) __builtin_shufflevector((__v2si)(__m), __extension__(__v2si){}, 0, 1, 2, 3), __count)), __extension__(__v2di){}, 0);
}
# 907 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_sra_pi32(__m64 __m, __m64 __count)
{
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_psrad128((__v4si)(__m128i) __builtin_shufflevector((__v2si)(__m), __extension__(__v2si){}, 0, 1, 2, 3), (__v4si)(__m128i) __builtin_shufflevector((__v2si)(__count), __extension__(__v2si){}, 0, 1, 2, 3))), __extension__(__v2di){}, 0);

}
# 931 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_srai_pi32(__m64 __m,
                                                              int __count) {
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_psradi128((__v4si)(__m128i) __builtin_shufflevector((__v2si)(__m), __extension__(__v2si){}, 0, 1, 2, 3), __count)), __extension__(__v2di){}, 0);
}
# 953 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_srl_pi16(__m64 __m, __m64 __count)
{
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_psrlw128((__v8hi)(__m128i) __builtin_shufflevector((__v2si)(__m), __extension__(__v2si){}, 0, 1, 2, 3), (__v8hi)(__m128i) __builtin_shufflevector((__v2si)(__count), __extension__(__v2si){}, 0, 1, 2, 3))), __extension__(__v2di){}, 0);

}
# 976 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_srli_pi16(__m64 __m,
                                                              int __count) {
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_psrlwi128((__v8hi)(__m128i) __builtin_shufflevector((__v2si)(__m), __extension__(__v2si){}, 0, 1, 2, 3), __count)), __extension__(__v2di){}, 0);
}
# 998 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_srl_pi32(__m64 __m, __m64 __count)
{
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_psrld128((__v4si)(__m128i) __builtin_shufflevector((__v2si)(__m), __extension__(__v2si){}, 0, 1, 2, 3), (__v4si)(__m128i) __builtin_shufflevector((__v2si)(__count), __extension__(__v2si){}, 0, 1, 2, 3))), __extension__(__v2di){}, 0);

}
# 1021 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_srli_pi32(__m64 __m,
                                                              int __count) {
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_psrldi128((__v4si)(__m128i) __builtin_shufflevector((__v2si)(__m), __extension__(__v2si){}, 0, 1, 2, 3), __count)), __extension__(__v2di){}, 0);
}
# 1040 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_srl_si64(__m64 __m, __m64 __count)
{
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_psrlq128((__v2di)(__m128i) __builtin_shufflevector((__v2si)(__m), __extension__(__v2si){}, 0, 1, 2, 3), (__v2di)(__m128i) __builtin_shufflevector((__v2si)(__count), __extension__(__v2si){}, 0, 1, 2, 3))), __extension__(__v2di){}, 0);

}
# 1062 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_srli_si64(__m64 __m,
                                                              int __count) {
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_psrlqi128((__v2di)(__m128i) __builtin_shufflevector((__v2si)(__m), __extension__(__v2si){}, 0, 1, 2, 3), __count)), __extension__(__v2di){}, 0);
}
# 1079 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_and_si64(__m64 __m1,
                                                             __m64 __m2) {
  return (__m64)(((__v1du)__m1) & ((__v1du)__m2));
}
# 1099 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_andnot_si64(__m64 __m1,
                                                                __m64 __m2) {
  return (__m64)(~((__v1du)__m1) & ((__v1du)__m2));
}
# 1116 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_or_si64(__m64 __m1,
                                                            __m64 __m2) {
  return (__m64)(((__v1du)__m1) | ((__v1du)__m2));
}
# 1133 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_xor_si64(__m64 __m1,
                                                             __m64 __m2) {
  return (__m64)(((__v1du)__m1) ^ ((__v1du)__m2));
}
# 1154 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cmpeq_pi8(__m64 __m1,
                                                              __m64 __m2) {
  return (__m64)(((__v8qi)__m1) == ((__v8qi)__m2));
}
# 1175 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cmpeq_pi16(__m64 __m1,
                                                               __m64 __m2) {
  return (__m64)(((__v4hi)__m1) == ((__v4hi)__m2));
}
# 1196 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cmpeq_pi32(__m64 __m1,
                                                               __m64 __m2) {
  return (__m64)(((__v2si)__m1) == ((__v2si)__m2));
}
# 1217 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cmpgt_pi8(__m64 __m1,
                                                              __m64 __m2) {


    return (__m64)((__v8qs)__m1 > (__v8qs)__m2);
}
# 1240 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cmpgt_pi16(__m64 __m1,
                                                               __m64 __m2) {
  return (__m64)((__v4hi)__m1 > (__v4hi)__m2);
}
# 1261 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cmpgt_pi32(__m64 __m1,
                                                               __m64 __m2) {
  return (__m64)((__v2si)__m1 > (__v2si)__m2);
}
# 1273 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_setzero_si64(void) {
  return __extension__(__m64){0LL};
}
# 1292 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_set_pi32(int __i1,
                                                             int __i0) {
  return __extension__(__m64)(__v2si){__i0, __i1};
}
# 1314 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_set_pi16(short __s3,
                                                             short __s2,
                                                             short __s1,
                                                             short __s0) {
  return __extension__(__m64)(__v4hi){__s0, __s1, __s2, __s3};
}
# 1346 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_set_pi8(char __b7, char __b6, char __b5, char __b4, char __b3, char __b2,
            char __b1, char __b0) {
  return __extension__(__m64)(__v8qi){__b0, __b1, __b2, __b3,
                                      __b4, __b5, __b6, __b7};
}
# 1366 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_set1_pi32(int __i) {
  return _mm_set_pi32(__i, __i);
}
# 1383 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_set1_pi16(short __w) {
  return _mm_set_pi16(__w, __w, __w, __w);
}
# 1399 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_set1_pi8(char __b) {
  return _mm_set_pi8(__b, __b, __b, __b, __b, __b, __b, __b);
}
# 1418 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_setr_pi32(int __i0,
                                                              int __i1) {
  return _mm_set_pi32(__i1, __i0);
}
# 1440 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_setr_pi16(short __w0,
                                                              short __w1,
                                                              short __w2,
                                                              short __w3) {
  return _mm_set_pi16(__w3, __w2, __w1, __w0);
}
# 1472 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_setr_pi8(char __b0, char __b1, char __b2, char __b3, char __b4, char __b5,
             char __b6, char __b7) {
  return _mm_set_pi8(__b7, __b6, __b5, __b4, __b3, __b2, __b1, __b0);
}
# 18 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h" 2

typedef float __v4sf __attribute__((__vector_size__(16)));
typedef float __m128 __attribute__((__vector_size__(16), __aligned__(16)));

typedef float __m128_u __attribute__((__vector_size__(16), __aligned__(1)));


typedef unsigned int __v4su __attribute__((__vector_size__(16)));
typedef unsigned short __v8hu __attribute__((__vector_size__(16)));
typedef unsigned char __v16qu __attribute__((__vector_size__(16)));




# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mm_malloc.h" 1
# 13 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mm_malloc.h"
# 1 "/usr/include/stdlib.h" 1 3 4
# 24 "/usr/include/stdlib.h" 3 4
# 1 "/usr/include/features.h" 1 3 4
# 345 "/usr/include/features.h" 3 4
# 1 "/usr/include/stdc-predef.h" 1 3 4
# 346 "/usr/include/features.h" 2 3 4
# 375 "/usr/include/features.h" 3 4
# 1 "/usr/include/sys/cdefs.h" 1 3 4
# 392 "/usr/include/sys/cdefs.h" 3 4
# 1 "/usr/include/bits/wordsize.h" 1 3 4
# 393 "/usr/include/sys/cdefs.h" 2 3 4
# 376 "/usr/include/features.h" 2 3 4
# 399 "/usr/include/features.h" 3 4
# 1 "/usr/include/gnu/stubs.h" 1 3 4
# 10 "/usr/include/gnu/stubs.h" 3 4
# 1 "/usr/include/gnu/stubs-64.h" 1 3 4
# 11 "/usr/include/gnu/stubs.h" 2 3 4
# 400 "/usr/include/features.h" 2 3 4
# 25 "/usr/include/stdlib.h" 2 3 4







# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h" 1 3 4
# 93 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h" 3 4
# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stddef_size_t.h" 1 3 4
# 18 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stddef_size_t.h" 3 4
typedef long unsigned int size_t;
# 94 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h" 2 3 4
# 103 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h" 3 4
# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stddef_wchar_t.h" 1 3 4
# 24 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stddef_wchar_t.h" 3 4
typedef int wchar_t;
# 104 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h" 2 3 4




# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stddef_null.h" 1 3 4
# 109 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h" 2 3 4
# 33 "/usr/include/stdlib.h" 2 3 4








# 1 "/usr/include/bits/waitflags.h" 1 3 4
# 42 "/usr/include/stdlib.h" 2 3 4
# 1 "/usr/include/bits/waitstatus.h" 1 3 4
# 64 "/usr/include/bits/waitstatus.h" 3 4
# 1 "/usr/include/endian.h" 1 3 4
# 36 "/usr/include/endian.h" 3 4
# 1 "/usr/include/bits/endian.h" 1 3 4
# 37 "/usr/include/endian.h" 2 3 4
# 60 "/usr/include/endian.h" 3 4
# 1 "/usr/include/bits/byteswap.h" 1 3 4
# 27 "/usr/include/bits/byteswap.h" 3 4
# 1 "/usr/include/bits/types.h" 1 3 4
# 27 "/usr/include/bits/types.h" 3 4
# 1 "/usr/include/bits/wordsize.h" 1 3 4
# 28 "/usr/include/bits/types.h" 2 3 4


typedef unsigned char __u_char;
typedef unsigned short int __u_short;
typedef unsigned int __u_int;
typedef unsigned long int __u_long;


typedef signed char __int8_t;
typedef unsigned char __uint8_t;
typedef signed short int __int16_t;
typedef unsigned short int __uint16_t;
typedef signed int __int32_t;
typedef unsigned int __uint32_t;

typedef signed long int __int64_t;
typedef unsigned long int __uint64_t;







typedef long int __quad_t;
typedef unsigned long int __u_quad_t;
# 130 "/usr/include/bits/types.h" 3 4
# 1 "/usr/include/bits/typesizes.h" 1 3 4
# 131 "/usr/include/bits/types.h" 2 3 4


typedef unsigned long int __dev_t;
typedef unsigned int __uid_t;
typedef unsigned int __gid_t;
typedef unsigned long int __ino_t;
typedef unsigned long int __ino64_t;
typedef unsigned int __mode_t;
typedef unsigned long int __nlink_t;
typedef long int __off_t;
typedef long int __off64_t;
typedef int __pid_t;
typedef struct { int __val[2]; } __fsid_t;
typedef long int __clock_t;
typedef unsigned long int __rlim_t;
typedef unsigned long int __rlim64_t;
typedef unsigned int __id_t;
typedef long int __time_t;
typedef unsigned int __useconds_t;
typedef long int __suseconds_t;

typedef int __daddr_t;
typedef int __key_t;


typedef int __clockid_t;


typedef void * __timer_t;


typedef long int __blksize_t;




typedef long int __blkcnt_t;
typedef long int __blkcnt64_t;


typedef unsigned long int __fsblkcnt_t;
typedef unsigned long int __fsblkcnt64_t;


typedef unsigned long int __fsfilcnt_t;
typedef unsigned long int __fsfilcnt64_t;


typedef long int __fsword_t;

typedef long int __ssize_t;


typedef long int __syscall_slong_t;

typedef unsigned long int __syscall_ulong_t;



typedef __off64_t __loff_t;
typedef __quad_t *__qaddr_t;
typedef char *__caddr_t;


typedef long int __intptr_t;


typedef unsigned int __socklen_t;
# 28 "/usr/include/bits/byteswap.h" 2 3 4
# 1 "/usr/include/bits/wordsize.h" 1 3 4
# 29 "/usr/include/bits/byteswap.h" 2 3 4






# 1 "/usr/include/bits/byteswap-16.h" 1 3 4
# 36 "/usr/include/bits/byteswap.h" 2 3 4
# 61 "/usr/include/endian.h" 2 3 4
# 65 "/usr/include/bits/waitstatus.h" 2 3 4

union wait
  {
    int w_status;
    struct
      {

 unsigned int __w_termsig:7;
 unsigned int __w_coredump:1;
 unsigned int __w_retcode:8;
 unsigned int:16;







      } __wait_terminated;
    struct
      {

 unsigned int __w_stopval:8;
 unsigned int __w_stopsig:8;
 unsigned int:16;






      } __wait_stopped;
  };
# 43 "/usr/include/stdlib.h" 2 3 4
# 67 "/usr/include/stdlib.h" 3 4
typedef union
  {
    union wait *__uptr;
    int *__iptr;
  } __WAIT_STATUS __attribute__ ((__transparent_union__));
# 97 "/usr/include/stdlib.h" 3 4
typedef struct
  {
    int quot;
    int rem;
  } div_t;



typedef struct
  {
    long int quot;
    long int rem;
  } ldiv_t;







__extension__ typedef struct
  {
    long long int quot;
    long long int rem;
  } lldiv_t;
# 139 "/usr/include/stdlib.h" 3 4
extern size_t __ctype_get_mb_cur_max (void) __attribute__ ((__nothrow__ )) ;




extern double atof (const char *__nptr)
     __attribute__ ((__nothrow__ )) __attribute__ ((__pure__)) __attribute__ ((__nonnull__ (1))) ;

extern int atoi (const char *__nptr)
     __attribute__ ((__nothrow__ )) __attribute__ ((__pure__)) __attribute__ ((__nonnull__ (1))) ;

extern long int atol (const char *__nptr)
     __attribute__ ((__nothrow__ )) __attribute__ ((__pure__)) __attribute__ ((__nonnull__ (1))) ;





__extension__ extern long long int atoll (const char *__nptr)
     __attribute__ ((__nothrow__ )) __attribute__ ((__pure__)) __attribute__ ((__nonnull__ (1))) ;





extern double strtod (const char *__restrict __nptr,
        char **__restrict __endptr)
     __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (1)));





extern float strtof (const char *__restrict __nptr,
       char **__restrict __endptr) __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (1)));

extern long double strtold (const char *__restrict __nptr,
       char **__restrict __endptr)
     __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (1)));





extern long int strtol (const char *__restrict __nptr,
   char **__restrict __endptr, int __base)
     __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (1)));

extern unsigned long int strtoul (const char *__restrict __nptr,
      char **__restrict __endptr, int __base)
     __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (1)));




__extension__
extern long long int strtoq (const char *__restrict __nptr,
        char **__restrict __endptr, int __base)
     __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (1)));

__extension__
extern unsigned long long int strtouq (const char *__restrict __nptr,
           char **__restrict __endptr, int __base)
     __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (1)));





__extension__
extern long long int strtoll (const char *__restrict __nptr,
         char **__restrict __endptr, int __base)
     __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (1)));

__extension__
extern unsigned long long int strtoull (const char *__restrict __nptr,
     char **__restrict __endptr, int __base)
     __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (1)));
# 305 "/usr/include/stdlib.h" 3 4
extern char *l64a (long int __n) __attribute__ ((__nothrow__ )) ;


extern long int a64l (const char *__s)
     __attribute__ ((__nothrow__ )) __attribute__ ((__pure__)) __attribute__ ((__nonnull__ (1))) ;




# 1 "/usr/include/sys/types.h" 1 3 4
# 33 "/usr/include/sys/types.h" 3 4
typedef __u_char u_char;
typedef __u_short u_short;
typedef __u_int u_int;
typedef __u_long u_long;
typedef __quad_t quad_t;
typedef __u_quad_t u_quad_t;
typedef __fsid_t fsid_t;




typedef __loff_t loff_t;



typedef __ino_t ino_t;
# 60 "/usr/include/sys/types.h" 3 4
typedef __dev_t dev_t;




typedef __gid_t gid_t;




typedef __mode_t mode_t;




typedef __nlink_t nlink_t;




typedef __uid_t uid_t;





typedef __off_t off_t;
# 98 "/usr/include/sys/types.h" 3 4
typedef __pid_t pid_t;





typedef __id_t id_t;




typedef __ssize_t ssize_t;





typedef __daddr_t daddr_t;
typedef __caddr_t caddr_t;





typedef __key_t key_t;
# 132 "/usr/include/sys/types.h" 3 4
# 1 "/usr/include/time.h" 1 3 4
# 59 "/usr/include/time.h" 3 4
typedef __clock_t clock_t;
# 75 "/usr/include/time.h" 3 4
typedef __time_t time_t;
# 91 "/usr/include/time.h" 3 4
typedef __clockid_t clockid_t;
# 103 "/usr/include/time.h" 3 4
typedef __timer_t timer_t;
# 133 "/usr/include/sys/types.h" 2 3 4
# 146 "/usr/include/sys/types.h" 3 4
# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h" 1 3 4
# 93 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h" 3 4
# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stddef_size_t.h" 1 3 4
# 94 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h" 2 3 4
# 147 "/usr/include/sys/types.h" 2 3 4



typedef unsigned long int ulong;
typedef unsigned short int ushort;
typedef unsigned int uint;
# 194 "/usr/include/sys/types.h" 3 4
typedef int int8_t __attribute__ ((__mode__ (__QI__)));
typedef int int16_t __attribute__ ((__mode__ (__HI__)));
typedef int int32_t __attribute__ ((__mode__ (__SI__)));
typedef int int64_t __attribute__ ((__mode__ (__DI__)));


typedef unsigned int u_int8_t __attribute__ ((__mode__ (__QI__)));
typedef unsigned int u_int16_t __attribute__ ((__mode__ (__HI__)));
typedef unsigned int u_int32_t __attribute__ ((__mode__ (__SI__)));
typedef unsigned int u_int64_t __attribute__ ((__mode__ (__DI__)));

typedef int register_t __attribute__ ((__mode__ (__word__)));
# 219 "/usr/include/sys/types.h" 3 4
# 1 "/usr/include/sys/select.h" 1 3 4
# 30 "/usr/include/sys/select.h" 3 4
# 1 "/usr/include/bits/select.h" 1 3 4
# 22 "/usr/include/bits/select.h" 3 4
# 1 "/usr/include/bits/wordsize.h" 1 3 4
# 23 "/usr/include/bits/select.h" 2 3 4
# 31 "/usr/include/sys/select.h" 2 3 4


# 1 "/usr/include/bits/sigset.h" 1 3 4
# 23 "/usr/include/bits/sigset.h" 3 4
typedef int __sig_atomic_t;




typedef struct
  {
    unsigned long int __val[(1024 / (8 * sizeof (unsigned long int)))];
  } __sigset_t;
# 34 "/usr/include/sys/select.h" 2 3 4



typedef __sigset_t sigset_t;





# 1 "/usr/include/time.h" 1 3 4
# 120 "/usr/include/time.h" 3 4
struct timespec
  {
    __time_t tv_sec;
    __syscall_slong_t tv_nsec;
  };
# 44 "/usr/include/sys/select.h" 2 3 4

# 1 "/usr/include/bits/time.h" 1 3 4
# 30 "/usr/include/bits/time.h" 3 4
struct timeval
  {
    __time_t tv_sec;
    __suseconds_t tv_usec;
  };
# 46 "/usr/include/sys/select.h" 2 3 4


typedef __suseconds_t suseconds_t;





typedef long int __fd_mask;
# 64 "/usr/include/sys/select.h" 3 4
typedef struct
  {






    __fd_mask __fds_bits[1024 / (8 * (int) sizeof (__fd_mask))];


  } fd_set;






typedef __fd_mask fd_mask;
# 106 "/usr/include/sys/select.h" 3 4
extern int select (int __nfds, fd_set *__restrict __readfds,
     fd_set *__restrict __writefds,
     fd_set *__restrict __exceptfds,
     struct timeval *__restrict __timeout);
# 118 "/usr/include/sys/select.h" 3 4
extern int pselect (int __nfds, fd_set *__restrict __readfds,
      fd_set *__restrict __writefds,
      fd_set *__restrict __exceptfds,
      const struct timespec *__restrict __timeout,
      const __sigset_t *__restrict __sigmask);
# 220 "/usr/include/sys/types.h" 2 3 4


# 1 "/usr/include/sys/sysmacros.h" 1 3 4
# 31 "/usr/include/sys/sysmacros.h" 3 4
__extension__
extern unsigned int gnu_dev_major (unsigned long long int __dev)
     __attribute__ ((__nothrow__ )) __attribute__ ((__const__));
__extension__
extern unsigned int gnu_dev_minor (unsigned long long int __dev)
     __attribute__ ((__nothrow__ )) __attribute__ ((__const__));
__extension__
extern unsigned long long int gnu_dev_makedev (unsigned int __major,
            unsigned int __minor)
     __attribute__ ((__nothrow__ )) __attribute__ ((__const__));
# 223 "/usr/include/sys/types.h" 2 3 4





typedef __blksize_t blksize_t;






typedef __blkcnt_t blkcnt_t;



typedef __fsblkcnt_t fsblkcnt_t;



typedef __fsfilcnt_t fsfilcnt_t;
# 270 "/usr/include/sys/types.h" 3 4
# 1 "/usr/include/bits/pthreadtypes.h" 1 3 4
# 21 "/usr/include/bits/pthreadtypes.h" 3 4
# 1 "/usr/include/bits/wordsize.h" 1 3 4
# 22 "/usr/include/bits/pthreadtypes.h" 2 3 4
# 60 "/usr/include/bits/pthreadtypes.h" 3 4
typedef unsigned long int pthread_t;


union pthread_attr_t
{
  char __size[56];
  long int __align;
};

typedef union pthread_attr_t pthread_attr_t;





typedef struct __pthread_internal_list
{
  struct __pthread_internal_list *__prev;
  struct __pthread_internal_list *__next;
} __pthread_list_t;
# 90 "/usr/include/bits/pthreadtypes.h" 3 4
typedef union
{
  struct __pthread_mutex_s
  {
    int __lock;
    unsigned int __count;
    int __owner;

    unsigned int __nusers;



    int __kind;

    short __spins;
    short __elision;
    __pthread_list_t __list;
# 124 "/usr/include/bits/pthreadtypes.h" 3 4
  } __data;
  char __size[40];
  long int __align;
} pthread_mutex_t;

typedef union
{
  char __size[4];
  int __align;
} pthread_mutexattr_t;




typedef union
{
  struct
  {
    int __lock;
    unsigned int __futex;
    __extension__ unsigned long long int __total_seq;
    __extension__ unsigned long long int __wakeup_seq;
    __extension__ unsigned long long int __woken_seq;
    void *__mutex;
    unsigned int __nwaiters;
    unsigned int __broadcast_seq;
  } __data;
  char __size[48];
  __extension__ long long int __align;
} pthread_cond_t;

typedef union
{
  char __size[4];
  int __align;
} pthread_condattr_t;



typedef unsigned int pthread_key_t;



typedef int pthread_once_t;





typedef union
{

  struct
  {
    int __lock;
    unsigned int __nr_readers;
    unsigned int __readers_wakeup;
    unsigned int __writer_wakeup;
    unsigned int __nr_readers_queued;
    unsigned int __nr_writers_queued;
    int __writer;
    int __shared;
    unsigned long int __pad1;
    unsigned long int __pad2;


    unsigned int __flags;

  } __data;
# 211 "/usr/include/bits/pthreadtypes.h" 3 4
  char __size[56];
  long int __align;
} pthread_rwlock_t;

typedef union
{
  char __size[8];
  long int __align;
} pthread_rwlockattr_t;





typedef volatile int pthread_spinlock_t;




typedef union
{
  char __size[32];
  long int __align;
} pthread_barrier_t;

typedef union
{
  char __size[4];
  int __align;
} pthread_barrierattr_t;
# 271 "/usr/include/sys/types.h" 2 3 4
# 315 "/usr/include/stdlib.h" 2 3 4






extern long int random (void) __attribute__ ((__nothrow__ ));


extern void srandom (unsigned int __seed) __attribute__ ((__nothrow__ ));





extern char *initstate (unsigned int __seed, char *__statebuf,
   size_t __statelen) __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (2)));



extern char *setstate (char *__statebuf) __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (1)));







struct random_data
  {
    int32_t *fptr;
    int32_t *rptr;
    int32_t *state;
    int rand_type;
    int rand_deg;
    int rand_sep;
    int32_t *end_ptr;
  };

extern int random_r (struct random_data *__restrict __buf,
       int32_t *__restrict __result) __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (1, 2)));

extern int srandom_r (unsigned int __seed, struct random_data *__buf)
     __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (2)));

extern int initstate_r (unsigned int __seed, char *__restrict __statebuf,
   size_t __statelen,
   struct random_data *__restrict __buf)
     __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (2, 4)));

extern int setstate_r (char *__restrict __statebuf,
         struct random_data *__restrict __buf)
     __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (1, 2)));






extern int rand (void) __attribute__ ((__nothrow__ ));

extern void srand (unsigned int __seed) __attribute__ ((__nothrow__ ));




extern int rand_r (unsigned int *__seed) __attribute__ ((__nothrow__ ));







extern double drand48 (void) __attribute__ ((__nothrow__ ));
extern double erand48 (unsigned short int __xsubi[3]) __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (1)));


extern long int lrand48 (void) __attribute__ ((__nothrow__ ));
extern long int nrand48 (unsigned short int __xsubi[3])
     __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (1)));


extern long int mrand48 (void) __attribute__ ((__nothrow__ ));
extern long int jrand48 (unsigned short int __xsubi[3])
     __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (1)));


extern void srand48 (long int __seedval) __attribute__ ((__nothrow__ ));
extern unsigned short int *seed48 (unsigned short int __seed16v[3])
     __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (1)));
extern void lcong48 (unsigned short int __param[7]) __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (1)));





struct drand48_data
  {
    unsigned short int __x[3];
    unsigned short int __old_x[3];
    unsigned short int __c;
    unsigned short int __init;
    unsigned long long int __a;
  };


extern int drand48_r (struct drand48_data *__restrict __buffer,
        double *__restrict __result) __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (1, 2)));
extern int erand48_r (unsigned short int __xsubi[3],
        struct drand48_data *__restrict __buffer,
        double *__restrict __result) __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (1, 2)));


extern int lrand48_r (struct drand48_data *__restrict __buffer,
        long int *__restrict __result)
     __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (1, 2)));
extern int nrand48_r (unsigned short int __xsubi[3],
        struct drand48_data *__restrict __buffer,
        long int *__restrict __result)
     __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (1, 2)));


extern int mrand48_r (struct drand48_data *__restrict __buffer,
        long int *__restrict __result)
     __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (1, 2)));
extern int jrand48_r (unsigned short int __xsubi[3],
        struct drand48_data *__restrict __buffer,
        long int *__restrict __result)
     __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (1, 2)));


extern int srand48_r (long int __seedval, struct drand48_data *__buffer)
     __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (2)));

extern int seed48_r (unsigned short int __seed16v[3],
       struct drand48_data *__buffer) __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (1, 2)));

extern int lcong48_r (unsigned short int __param[7],
        struct drand48_data *__buffer)
     __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (1, 2)));
# 465 "/usr/include/stdlib.h" 3 4
extern void *malloc (size_t __size) __attribute__ ((__nothrow__ )) __attribute__ ((__malloc__)) ;

extern void *calloc (size_t __nmemb, size_t __size)
     __attribute__ ((__nothrow__ )) __attribute__ ((__malloc__)) ;
# 479 "/usr/include/stdlib.h" 3 4
extern void *realloc (void *__ptr, size_t __size)
     __attribute__ ((__nothrow__ )) __attribute__ ((__warn_unused_result__));

extern void free (void *__ptr) __attribute__ ((__nothrow__ ));




extern void cfree (void *__ptr) __attribute__ ((__nothrow__ ));



# 1 "/usr/include/alloca.h" 1 3 4
# 24 "/usr/include/alloca.h" 3 4
# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h" 1 3 4
# 93 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h" 3 4
# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stddef_size_t.h" 1 3 4
# 94 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h" 2 3 4
# 25 "/usr/include/alloca.h" 2 3 4







extern void *alloca (size_t __size) __attribute__ ((__nothrow__ ));
# 492 "/usr/include/stdlib.h" 2 3 4





extern void *valloc (size_t __size) __attribute__ ((__nothrow__ )) __attribute__ ((__malloc__)) ;




extern int posix_memalign (void **__memptr, size_t __alignment, size_t __size)
     __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (1))) ;




extern void *aligned_alloc (size_t __alignment, size_t __size)
     __attribute__ ((__nothrow__ )) __attribute__ ((__malloc__, __alloc_size__ (2)));




extern void abort (void) __attribute__ ((__nothrow__ )) __attribute__ ((__noreturn__));



extern int atexit (void (*__func) (void)) __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (1)));







extern int at_quick_exit (void (*__func) (void)) __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (1)));







extern int on_exit (void (*__func) (int __status, void *__arg), void *__arg)
     __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (1)));






extern void exit (int __status) __attribute__ ((__nothrow__ )) __attribute__ ((__noreturn__));





extern void quick_exit (int __status) __attribute__ ((__nothrow__ )) __attribute__ ((__noreturn__));







extern void _Exit (int __status) __attribute__ ((__nothrow__ )) __attribute__ ((__noreturn__));






extern char *getenv (const char *__name) __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (1))) ;
# 577 "/usr/include/stdlib.h" 3 4
extern int putenv (char *__string) __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (1)));





extern int setenv (const char *__name, const char *__value, int __replace)
     __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (2)));


extern int unsetenv (const char *__name) __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (1)));






extern int clearenv (void) __attribute__ ((__nothrow__ ));
# 605 "/usr/include/stdlib.h" 3 4
extern char *mktemp (char *__template) __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (1)));
# 619 "/usr/include/stdlib.h" 3 4
extern int mkstemp (char *__template) __attribute__ ((__nonnull__ (1))) ;
# 641 "/usr/include/stdlib.h" 3 4
extern int mkstemps (char *__template, int __suffixlen) __attribute__ ((__nonnull__ (1))) ;
# 662 "/usr/include/stdlib.h" 3 4
extern char *mkdtemp (char *__template) __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (1))) ;
# 716 "/usr/include/stdlib.h" 3 4
extern int system (const char *__command) ;
# 733 "/usr/include/stdlib.h" 3 4
extern char *realpath (const char *__restrict __name,
         char *__restrict __resolved) __attribute__ ((__nothrow__ )) ;






typedef int (*__compar_fn_t) (const void *, const void *);
# 754 "/usr/include/stdlib.h" 3 4
extern void *bsearch (const void *__key, const void *__base,
        size_t __nmemb, size_t __size, __compar_fn_t __compar)
     __attribute__ ((__nonnull__ (1, 2, 5))) ;



extern void qsort (void *__base, size_t __nmemb, size_t __size,
     __compar_fn_t __compar) __attribute__ ((__nonnull__ (1, 4)));
# 770 "/usr/include/stdlib.h" 3 4
extern int abs (int __x) __attribute__ ((__nothrow__ )) __attribute__ ((__const__)) ;
extern long int labs (long int __x) __attribute__ ((__nothrow__ )) __attribute__ ((__const__)) ;



__extension__ extern long long int llabs (long long int __x)
     __attribute__ ((__nothrow__ )) __attribute__ ((__const__)) ;







extern div_t div (int __numer, int __denom)
     __attribute__ ((__nothrow__ )) __attribute__ ((__const__)) ;
extern ldiv_t ldiv (long int __numer, long int __denom)
     __attribute__ ((__nothrow__ )) __attribute__ ((__const__)) ;




__extension__ extern lldiv_t lldiv (long long int __numer,
        long long int __denom)
     __attribute__ ((__nothrow__ )) __attribute__ ((__const__)) ;
# 807 "/usr/include/stdlib.h" 3 4
extern char *ecvt (double __value, int __ndigit, int *__restrict __decpt,
     int *__restrict __sign) __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (3, 4))) ;




extern char *fcvt (double __value, int __ndigit, int *__restrict __decpt,
     int *__restrict __sign) __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (3, 4))) ;




extern char *gcvt (double __value, int __ndigit, char *__buf)
     __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (3))) ;




extern char *qecvt (long double __value, int __ndigit,
      int *__restrict __decpt, int *__restrict __sign)
     __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (3, 4))) ;
extern char *qfcvt (long double __value, int __ndigit,
      int *__restrict __decpt, int *__restrict __sign)
     __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (3, 4))) ;
extern char *qgcvt (long double __value, int __ndigit, char *__buf)
     __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (3))) ;




extern int ecvt_r (double __value, int __ndigit, int *__restrict __decpt,
     int *__restrict __sign, char *__restrict __buf,
     size_t __len) __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (3, 4, 5)));
extern int fcvt_r (double __value, int __ndigit, int *__restrict __decpt,
     int *__restrict __sign, char *__restrict __buf,
     size_t __len) __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (3, 4, 5)));

extern int qecvt_r (long double __value, int __ndigit,
      int *__restrict __decpt, int *__restrict __sign,
      char *__restrict __buf, size_t __len)
     __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (3, 4, 5)));
extern int qfcvt_r (long double __value, int __ndigit,
      int *__restrict __decpt, int *__restrict __sign,
      char *__restrict __buf, size_t __len)
     __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (3, 4, 5)));







extern int mblen (const char *__s, size_t __n) __attribute__ ((__nothrow__ )) ;


extern int mbtowc (wchar_t *__restrict __pwc,
     const char *__restrict __s, size_t __n) __attribute__ ((__nothrow__ )) ;


extern int wctomb (char *__s, wchar_t __wchar) __attribute__ ((__nothrow__ )) ;



extern size_t mbstowcs (wchar_t *__restrict __pwcs,
   const char *__restrict __s, size_t __n) __attribute__ ((__nothrow__ ));

extern size_t wcstombs (char *__restrict __s,
   const wchar_t *__restrict __pwcs, size_t __n)
     __attribute__ ((__nothrow__ ));
# 884 "/usr/include/stdlib.h" 3 4
extern int rpmatch (const char *__response) __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (1))) ;
# 895 "/usr/include/stdlib.h" 3 4
extern int getsubopt (char **__restrict __optionp,
        char *const *__restrict __tokens,
        char **__restrict __valuep)
     __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (1, 2, 3))) ;
# 947 "/usr/include/stdlib.h" 3 4
extern int getloadavg (double __loadavg[], int __nelem)
     __attribute__ ((__nothrow__ )) __attribute__ ((__nonnull__ (1)));


# 1 "/usr/include/bits/stdlib-float.h" 1 3 4
# 952 "/usr/include/stdlib.h" 2 3 4
# 14 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mm_malloc.h" 2





extern int posix_memalign(void **__memptr, size_t __alignment, size_t __size);
# 30 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/mm_malloc.h"
static __inline__ void *__attribute__((__always_inline__, __nodebug__,
                                       __malloc__, __alloc_size__(1),
                                       __alloc_align__(2)))
_mm_malloc(size_t __size, size_t __align) {
  if (__align == 1) {
    return malloc(__size);
  }

  if (!(__align & (__align - 1)) && __align < sizeof(void *))
    __align = sizeof(void *);

  void *__mallocedMemory;





  if (posix_memalign(&__mallocedMemory, __align, __size))
    return 0;


  return __mallocedMemory;
}

static __inline__ void __attribute__((__always_inline__, __nodebug__))
_mm_free(void *__p)
{





  free(__p);

}
# 33 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h" 2
# 78 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_add_ss(__m128 __a, __m128 __b) {
  __a[0] += __b[0];
  return __a;
}
# 97 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_add_ps(__m128 __a, __m128 __b) {
  return (__m128)((__v4sf)__a + (__v4sf)__b);
}
# 118 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_sub_ss(__m128 __a, __m128 __b) {
  __a[0] -= __b[0];
  return __a;
}
# 138 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_sub_ps(__m128 __a, __m128 __b) {
  return (__m128)((__v4sf)__a - (__v4sf)__b);
}
# 159 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_mul_ss(__m128 __a, __m128 __b) {
  __a[0] *= __b[0];
  return __a;
}
# 178 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_mul_ps(__m128 __a, __m128 __b) {
  return (__m128)((__v4sf)__a * (__v4sf)__b);
}
# 199 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_div_ss(__m128 __a, __m128 __b) {
  __a[0] /= __b[0];
  return __a;
}
# 217 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_div_ps(__m128 __a, __m128 __b) {
  return (__m128)((__v4sf)__a / (__v4sf)__b);
}
# 234 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128))) _mm_sqrt_ss(__m128 __a) {
  __a[0] = __builtin_elementwise_sqrt(__a[0]);
  return __a;
}
# 250 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128))) _mm_sqrt_ps(__m128 __a) {
  return __builtin_elementwise_sqrt(__a);
}
# 266 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_rcp_ss(__m128 __a)
{
  return (__m128)__builtin_ia32_rcpss((__v4sf)__a);
}
# 283 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_rcp_ps(__m128 __a)
{
  return (__m128)__builtin_ia32_rcpps((__v4sf)__a);
}
# 302 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_rsqrt_ss(__m128 __a)
{
  return __builtin_ia32_rsqrtss((__v4sf)__a);
}
# 319 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_rsqrt_ps(__m128 __a)
{
  return __builtin_ia32_rsqrtps((__v4sf)__a);
}
# 344 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128))) _mm_min_ss(__m128 __a, __m128 __b) {
  return __builtin_ia32_minss((__v4sf)__a, (__v4sf)__b);
}
# 363 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128))) _mm_min_ps(__m128 __a,
                                                                 __m128 __b) {
  return __builtin_ia32_minps((__v4sf)__a, (__v4sf)__b);
}
# 387 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128))) _mm_max_ss(__m128 __a, __m128 __b) {
  return __builtin_ia32_maxss((__v4sf)__a, (__v4sf)__b);
}
# 406 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128))) _mm_max_ps(__m128 __a,
                                                                 __m128 __b) {
  return __builtin_ia32_maxps((__v4sf)__a, (__v4sf)__b);
}
# 423 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_and_ps(__m128 __a, __m128 __b) {
  return (__m128)((__v4su)__a & (__v4su)__b);
}
# 444 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_andnot_ps(__m128 __a, __m128 __b) {
  return (__m128)(~(__v4su)__a & (__v4su)__b);
}
# 461 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_or_ps(__m128 __a, __m128 __b) {
  return (__m128)((__v4su)__a | (__v4su)__b);
}
# 479 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_xor_ps(__m128 __a, __m128 __b) {
  return (__m128)((__v4su)__a ^ (__v4su)__b);
}
# 503 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_cmpeq_ss(__m128 __a, __m128 __b)
{
  return (__m128)__builtin_ia32_cmpeqss((__v4sf)__a, (__v4sf)__b);
}
# 524 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_cmpeq_ps(__m128 __a, __m128 __b)
{
  return (__m128)__builtin_ia32_cmpeqps((__v4sf)__a, (__v4sf)__b);
}
# 550 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_cmplt_ss(__m128 __a, __m128 __b)
{
  return (__m128)__builtin_ia32_cmpltss((__v4sf)__a, (__v4sf)__b);
}
# 572 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_cmplt_ps(__m128 __a, __m128 __b)
{
  return (__m128)__builtin_ia32_cmpltps((__v4sf)__a, (__v4sf)__b);
}
# 598 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_cmple_ss(__m128 __a, __m128 __b)
{
  return (__m128)__builtin_ia32_cmpless((__v4sf)__a, (__v4sf)__b);
}
# 620 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_cmple_ps(__m128 __a, __m128 __b)
{
  return (__m128)__builtin_ia32_cmpleps((__v4sf)__a, (__v4sf)__b);
}
# 646 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_cmpgt_ss(__m128 __a, __m128 __b)
{
  return (__m128)__builtin_shufflevector((__v4sf)__a,
                                         (__v4sf)__builtin_ia32_cmpltss((__v4sf)__b, (__v4sf)__a),
                                         4, 1, 2, 3);
}
# 670 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_cmpgt_ps(__m128 __a, __m128 __b)
{
  return (__m128)__builtin_ia32_cmpltps((__v4sf)__b, (__v4sf)__a);
}
# 696 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_cmpge_ss(__m128 __a, __m128 __b)
{
  return (__m128)__builtin_shufflevector((__v4sf)__a,
                                         (__v4sf)__builtin_ia32_cmpless((__v4sf)__b, (__v4sf)__a),
                                         4, 1, 2, 3);
}
# 720 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_cmpge_ps(__m128 __a, __m128 __b)
{
  return (__m128)__builtin_ia32_cmpleps((__v4sf)__b, (__v4sf)__a);
}
# 746 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_cmpneq_ss(__m128 __a, __m128 __b)
{
  return (__m128)__builtin_ia32_cmpneqss((__v4sf)__a, (__v4sf)__b);
}
# 768 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_cmpneq_ps(__m128 __a, __m128 __b)
{
  return (__m128)__builtin_ia32_cmpneqps((__v4sf)__a, (__v4sf)__b);
}
# 795 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_cmpnlt_ss(__m128 __a, __m128 __b)
{
  return (__m128)__builtin_ia32_cmpnltss((__v4sf)__a, (__v4sf)__b);
}
# 818 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_cmpnlt_ps(__m128 __a, __m128 __b)
{
  return (__m128)__builtin_ia32_cmpnltps((__v4sf)__a, (__v4sf)__b);
}
# 845 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_cmpnle_ss(__m128 __a, __m128 __b)
{
  return (__m128)__builtin_ia32_cmpnless((__v4sf)__a, (__v4sf)__b);
}
# 868 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_cmpnle_ps(__m128 __a, __m128 __b)
{
  return (__m128)__builtin_ia32_cmpnleps((__v4sf)__a, (__v4sf)__b);
}
# 895 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_cmpngt_ss(__m128 __a, __m128 __b)
{
  return (__m128)__builtin_shufflevector((__v4sf)__a,
                                         (__v4sf)__builtin_ia32_cmpnltss((__v4sf)__b, (__v4sf)__a),
                                         4, 1, 2, 3);
}
# 920 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_cmpngt_ps(__m128 __a, __m128 __b)
{
  return (__m128)__builtin_ia32_cmpnltps((__v4sf)__b, (__v4sf)__a);
}
# 947 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_cmpnge_ss(__m128 __a, __m128 __b)
{
  return (__m128)__builtin_shufflevector((__v4sf)__a,
                                         (__v4sf)__builtin_ia32_cmpnless((__v4sf)__b, (__v4sf)__a),
                                         4, 1, 2, 3);
}
# 972 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_cmpnge_ps(__m128 __a, __m128 __b)
{
  return (__m128)__builtin_ia32_cmpnleps((__v4sf)__b, (__v4sf)__a);
}
# 999 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_cmpord_ss(__m128 __a, __m128 __b)
{
  return (__m128)__builtin_ia32_cmpordss((__v4sf)__a, (__v4sf)__b);
}
# 1023 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_cmpord_ps(__m128 __a, __m128 __b)
{
  return (__m128)__builtin_ia32_cmpordps((__v4sf)__a, (__v4sf)__b);
}
# 1050 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_cmpunord_ss(__m128 __a, __m128 __b)
{
  return (__m128)__builtin_ia32_cmpunordss((__v4sf)__a, (__v4sf)__b);
}
# 1074 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_cmpunord_ps(__m128 __a, __m128 __b)
{
  return (__m128)__builtin_ia32_cmpunordps((__v4sf)__a, (__v4sf)__b);
}
# 1098 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ int __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_comieq_ss(__m128 __a, __m128 __b)
{
  return __builtin_ia32_comieq((__v4sf)__a, (__v4sf)__b);
}
# 1123 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ int __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_comilt_ss(__m128 __a, __m128 __b)
{
  return __builtin_ia32_comilt((__v4sf)__a, (__v4sf)__b);
}
# 1147 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ int __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_comile_ss(__m128 __a, __m128 __b)
{
  return __builtin_ia32_comile((__v4sf)__a, (__v4sf)__b);
}
# 1171 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ int __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_comigt_ss(__m128 __a, __m128 __b)
{
  return __builtin_ia32_comigt((__v4sf)__a, (__v4sf)__b);
}
# 1195 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ int __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_comige_ss(__m128 __a, __m128 __b)
{
  return __builtin_ia32_comige((__v4sf)__a, (__v4sf)__b);
}
# 1219 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ int __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_comineq_ss(__m128 __a, __m128 __b)
{
  return __builtin_ia32_comineq((__v4sf)__a, (__v4sf)__b);
}
# 1242 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ int __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_ucomieq_ss(__m128 __a, __m128 __b)
{
  return __builtin_ia32_ucomieq((__v4sf)__a, (__v4sf)__b);
}
# 1266 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ int __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_ucomilt_ss(__m128 __a, __m128 __b)
{
  return __builtin_ia32_ucomilt((__v4sf)__a, (__v4sf)__b);
}
# 1290 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ int __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_ucomile_ss(__m128 __a, __m128 __b)
{
  return __builtin_ia32_ucomile((__v4sf)__a, (__v4sf)__b);
}
# 1314 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ int __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_ucomigt_ss(__m128 __a, __m128 __b)
{
  return __builtin_ia32_ucomigt((__v4sf)__a, (__v4sf)__b);
}
# 1338 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ int __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_ucomige_ss(__m128 __a, __m128 __b)
{
  return __builtin_ia32_ucomige((__v4sf)__a, (__v4sf)__b);
}
# 1361 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ int __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_ucomineq_ss(__m128 __a, __m128 __b)
{
  return __builtin_ia32_ucomineq((__v4sf)__a, (__v4sf)__b);
}
# 1383 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ int __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_cvtss_si32(__m128 __a)
{
  return __builtin_ia32_cvtss2si((__v4sf)__a);
}
# 1405 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ int __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_cvt_ss2si(__m128 __a)
{
  return _mm_cvtss_si32(__a);
}
# 1429 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ long long __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_cvtss_si64(__m128 __a)
{
  return __builtin_ia32_cvtss2si64((__v4sf)__a);
}
# 1451 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_cvtps_pi32(__m128 __a)
{
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_cvtps2dq((__v4sf)(__m128i) __builtin_shufflevector((__v4si)(__a), __extension__(__v4si){}, 0, 1, 4, 5))), __extension__(__v2di){}, 0);
}
# 1471 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_cvt_ps2pi(__m128 __a)
{
  return _mm_cvtps_pi32(__a);
}
# 1493 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ int __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_cvttss_si32(__m128 __a)
{
  return __builtin_ia32_cvttss2si((__v4sf)__a);
}
# 1515 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ int __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_cvtt_ss2si(__m128 __a)
{
  return _mm_cvttss_si32(__a);
}
# 1538 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ long long __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_cvttss_si64(__m128 __a)
{
  return __builtin_ia32_cvttss2si64((__v4sf)__a);
}
# 1561 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_cvttps_pi32(__m128 __a)
{
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_cvttps2dq((__v4sf)(__m128i) __builtin_shufflevector((__v4si)(__a), __extension__(__v4si){}, 0, 1, 4, 5))), __extension__(__v2di){}, 0);
}
# 1582 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_cvtt_ps2pi(__m128 __a)
{
  return _mm_cvttps_pi32(__a);
}
# 1604 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128))) _mm_cvtsi32_ss(__m128 __a,
                                                                     int __b) {
  __a[0] = __b;
  return __a;
}
# 1626 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128))) _mm_cvt_si2ss(__m128 __a,
                                                                    int __b) {
  return _mm_cvtsi32_ss(__a, __b);
}
# 1649 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_cvtsi64_ss(__m128 __a, long long __b) {
  __a[0] = __b;
  return __a;
}
# 1674 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_cvtpi32_ps(__m128 __a, __m64 __b)
{
  return (__m128)__builtin_shufflevector(
      (__v4sf)__a,
      __builtin_convertvector((__v4si)(__m128i) __builtin_shufflevector((__v2si)(__b), __extension__(__v2si){}, 0, 1, 2, 3), __v4sf),
      4, 5, 2, 3);
}
# 1700 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_cvt_pi2ps(__m128 __a, __m64 __b)
{
  return _mm_cvtpi32_ps(__a, __b);
}
# 1717 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ float __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_cvtss_f32(__m128 __a) {
  return __a[0];
}
# 1737 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_loadh_pi(__m128 __a, const __m64 *__p)
{
  typedef float __mm_loadh_pi_v2f32 __attribute__((__vector_size__(8)));
  struct __mm_loadh_pi_struct {
    __mm_loadh_pi_v2f32 __u;
  } __attribute__((__packed__, __may_alias__));
  __mm_loadh_pi_v2f32 __b = ((const struct __mm_loadh_pi_struct*)__p)->__u;
  __m128 __bb = __builtin_shufflevector(__b, __b, 0, 1, 0, 1);
  return __builtin_shufflevector(__a, __bb, 0, 1, 4, 5);
}
# 1764 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_loadl_pi(__m128 __a, const __m64 *__p)
{
  typedef float __mm_loadl_pi_v2f32 __attribute__((__vector_size__(8)));
  struct __mm_loadl_pi_struct {
    __mm_loadl_pi_v2f32 __u;
  } __attribute__((__packed__, __may_alias__));
  __mm_loadl_pi_v2f32 __b = ((const struct __mm_loadl_pi_struct*)__p)->__u;
  __m128 __bb = __builtin_shufflevector(__b, __b, 0, 1, 0, 1);
  return __builtin_shufflevector(__a, __bb, 4, 5, 2, 3);
}
# 1791 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_load_ss(const float *__p)
{
  struct __mm_load_ss_struct {
    float __u;
  } __attribute__((__packed__, __may_alias__));
  float __u = ((const struct __mm_load_ss_struct*)__p)->__u;
  return __extension__ (__m128){ __u, 0, 0, 0 };
}
# 1813 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_load1_ps(const float *__p)
{
  struct __mm_load1_ps_struct {
    float __u;
  } __attribute__((__packed__, __may_alias__));
  float __u = ((const struct __mm_load1_ps_struct*)__p)->__u;
  return __extension__ (__m128){ __u, __u, __u, __u };
}
# 1836 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_load_ps(const float *__p)
{
  return *(const __m128*)__p;
}
# 1853 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_loadu_ps(const float *__p)
{
  struct __loadu_ps {
    __m128_u __v;
  } __attribute__((__packed__, __may_alias__));
  return ((const struct __loadu_ps*)__p)->__v;
}
# 1875 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_loadr_ps(const float *__p)
{
  __m128 __a = _mm_load_ps(__p);
  return __builtin_shufflevector((__v4sf)__a, (__v4sf)__a, 3, 2, 1, 0);
}
# 1889 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_undefined_ps(void)
{
  return (__m128)__builtin_ia32_undef128();
}
# 1909 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_set_ss(float __w) {
  return __extension__ (__m128){ __w, 0.0f, 0.0f, 0.0f };
}
# 1926 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_set1_ps(float __w) {
  return __extension__ (__m128){ __w, __w, __w, __w };
}
# 1944 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_set_ps1(float __w) {
    return _mm_set1_ps(__w);
}
# 1970 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_set_ps(float __z, float __y, float __x, float __w) {
  return __extension__ (__m128){ __w, __x, __y, __z };
}
# 1997 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_setr_ps(float __z, float __y, float __x, float __w) {
  return __extension__ (__m128){ __z, __y, __x, __w };
}
# 2011 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_setzero_ps(void) {
  return __extension__ (__m128){ 0.0f, 0.0f, 0.0f, 0.0f };
}
# 2027 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ void __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_storeh_pi(__m64 *__p, __m128 __a)
{
  typedef float __mm_storeh_pi_v2f32 __attribute__((__vector_size__(8)));
  struct __mm_storeh_pi_struct {
    __mm_storeh_pi_v2f32 __u;
  } __attribute__((__packed__, __may_alias__));
  ((struct __mm_storeh_pi_struct*)__p)->__u = __builtin_shufflevector(__a, __a, 2, 3);
}
# 2048 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ void __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_storel_pi(__m64 *__p, __m128 __a)
{
  typedef float __mm_storeh_pi_v2f32 __attribute__((__vector_size__(8)));
  struct __mm_storeh_pi_struct {
    __mm_storeh_pi_v2f32 __u;
  } __attribute__((__packed__, __may_alias__));
  ((struct __mm_storeh_pi_struct*)__p)->__u = __builtin_shufflevector(__a, __a, 0, 1);
}
# 2069 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ void __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_store_ss(float *__p, __m128 __a)
{
  struct __mm_store_ss_struct {
    float __u;
  } __attribute__((__packed__, __may_alias__));
  ((struct __mm_store_ss_struct*)__p)->__u = __a[0];
}
# 2090 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ void __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_storeu_ps(float *__p, __m128 __a)
{
  struct __storeu_ps {
    __m128_u __v;
  } __attribute__((__packed__, __may_alias__));
  ((struct __storeu_ps*)__p)->__v = __a;
}
# 2111 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ void __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_store_ps(float *__p, __m128 __a)
{
  *(__m128*)__p = __a;
}
# 2130 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ void __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_store1_ps(float *__p, __m128 __a)
{
  __a = __builtin_shufflevector((__v4sf)__a, (__v4sf)__a, 0, 0, 0, 0);
  _mm_store_ps(__p, __a);
}
# 2150 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ void __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_store_ps1(float *__p, __m128 __a)
{
  _mm_store1_ps(__p, __a);
}
# 2169 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ void __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_storer_ps(float *__p, __m128 __a)
{
  __a = __builtin_shufflevector((__v4sf)__a, (__v4sf)__a, 3, 2, 1, 0);
  _mm_store_ps(__p, __a);
}
# 2228 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ void __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_stream_pi(void *__p, __m64 __a)
{
  __builtin_nontemporal_store(__a, (__m64 *)__p);
}
# 2247 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ void __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_stream_ps(void *__p, __m128 __a)
{
  __builtin_nontemporal_store((__v4sf)__a, (__v4sf*)__p);
}
# 2266 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
void _mm_sfence(void);
# 2339 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_max_pi16(__m64 __a, __m64 __b) {
  return (__m64)__builtin_elementwise_max((__v4hi)__a, (__v4hi)__b);
}
# 2357 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_max_pu8(__m64 __a, __m64 __b) {
  return (__m64)__builtin_elementwise_max((__v8qu)__a, (__v8qu)__b);
}
# 2375 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_min_pi16(__m64 __a, __m64 __b) {
  return (__m64)__builtin_elementwise_min((__v4hi)__a, (__v4hi)__b);
}
# 2393 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_min_pu8(__m64 __a, __m64 __b) {
  return (__m64)__builtin_elementwise_min((__v8qu)__a, (__v8qu)__b);
}
# 2410 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ int __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_movemask_pi8(__m64 __a) {
  return __builtin_ia32_pmovmskb128((__v16qi)(__m128i) __builtin_shufflevector((__v2si)(__a), __extension__(__v2si){}, 0, 1, 2, 3));
}
# 2428 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_mulhi_pu16(__m64 __a, __m64 __b)
{
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_pmulhuw128((__v8hu)(__m128i) __builtin_shufflevector((__v2si)(__a), __extension__(__v2si){}, 0, 1, 2, 3), (__v8hu)(__m128i) __builtin_shufflevector((__v2si)(__b), __extension__(__v2si){}, 0, 1, 2, 3))), __extension__(__v2di){}, 0);

}
# 2497 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ void __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_maskmove_si64(__m64 __d, __m64 __n, char *__p)
{




  __m128i __d128 = (__m128i) __builtin_shufflevector((__v2si)(__d), __extension__(__v2si){}, 0, 1, -1, -1);
  __m128i __n128 = (__m128i) __builtin_shufflevector((__v2si)(__n), __extension__(__v2si){}, 0, 1, 2, 3);
  if (((long unsigned int)__p & 0xfff) >= 4096-15 &&
      ((long unsigned int)__p & 0xfff) <= 4096-8) {


    __p -= 8;
    __d128 = (__m128i)__builtin_ia32_pslldqi128_byteshift((__v16qi)__d128, 8);
    __n128 = (__m128i)__builtin_ia32_pslldqi128_byteshift((__v16qi)__n128, 8);
  }

  __builtin_ia32_maskmovdqu((__v16qi)__d128, (__v16qi)__n128, __p);
}
# 2531 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_avg_pu8(__m64 __a, __m64 __b) {
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_pavgb128((__v16qu)(__m128i) __builtin_shufflevector((__v2si)(__a), __extension__(__v2si){}, 0, 1, 2, 3), (__v16qu)(__m128i) __builtin_shufflevector((__v2si)(__b), __extension__(__v2si){}, 0, 1, 2, 3))), __extension__(__v2di){}, 0);

}
# 2550 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_avg_pu16(__m64 __a, __m64 __b) {
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_pavgw128((__v8hu)(__m128i) __builtin_shufflevector((__v2si)(__a), __extension__(__v2si){}, 0, 1, 2, 3), (__v8hu)(__m128i) __builtin_shufflevector((__v2si)(__b), __extension__(__v2si){}, 0, 1, 2, 3))), __extension__(__v2di){}, 0);

}
# 2572 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_sad_pu8(__m64 __a, __m64 __b)
{
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_psadbw128((__v16qi)(__m128i) __builtin_shufflevector((__v2si)(__a), __extension__(__v2si){}, 0, 1, 2, 3), (__v16qi)(__m128i) __builtin_shufflevector((__v2si)(__b), __extension__(__v2si){}, 0, 1, 2, 3))), __extension__(__v2di){}, 0);

}
# 2633 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
unsigned int _mm_getcsr(void);
# 2687 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
void _mm_setcsr(unsigned int __i);
# 2752 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_unpackhi_ps(__m128 __a, __m128 __b) {
  return __builtin_shufflevector((__v4sf)__a, (__v4sf)__b, 2, 6, 3, 7);
}
# 2773 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_unpacklo_ps(__m128 __a, __m128 __b) {
  return __builtin_shufflevector((__v4sf)__a, (__v4sf)__b, 0, 4, 1, 5);
}
# 2794 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_move_ss(__m128 __a, __m128 __b) {
  __a[0] = __b[0];
  return __a;
}
# 2815 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_movehl_ps(__m128 __a, __m128 __b) {
  return __builtin_shufflevector((__v4sf)__a, (__v4sf)__b, 6, 7, 2, 3);
}
# 2835 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128)))
_mm_movelh_ps(__m128 __a, __m128 __b) {
  return __builtin_shufflevector((__v4sf)__a, (__v4sf)__b, 0, 1, 4, 5);
}
# 2852 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_cvtpi16_ps(__m64 __a)
{
  return __builtin_convertvector((__v4hi)__a, __v4sf);
}
# 2870 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_cvtpu16_ps(__m64 __a)
{
  return __builtin_convertvector((__v4hu)__a, __v4sf);
}
# 2888 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_cvtpi8_ps(__m64 __a)
{
  return __builtin_convertvector(
      __builtin_shufflevector((__v8qs)__a, __extension__ (__v8qs){},
                              0, 1, 2, 3), __v4sf);
}
# 2909 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_cvtpu8_ps(__m64 __a)
{
  return __builtin_convertvector(
      __builtin_shufflevector((__v8qu)__a, __extension__ (__v8qu){},
                              0, 1, 2, 3), __v4sf);
}
# 2933 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_cvtpi32x2_ps(__m64 __a, __m64 __b)
{
  return __builtin_convertvector(
      __builtin_shufflevector((__v2si)__a, (__v2si)__b,
                              0, 1, 2, 3), __v4sf);
}
# 2958 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_cvtps_pi16(__m128 __a)
{
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_packssdw128( (__v4si)__builtin_ia32_cvtps2dq((__v4sf)__a), (__v4si)_mm_setzero_ps())), __extension__(__v2di){}, 0);

}
# 2983 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_cvtps_pi8(__m128 __a)
{
  __m64 __b, __c;

  __b = _mm_cvtps_pi16(__a);
  __c = _mm_setzero_si64();

  return _mm_packs_pi16(__b, __c);
}
# 3008 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
static __inline__ int __attribute__((__always_inline__, __nodebug__, __target__("sse"), __min_vector_width__(128))) _mm_movemask_ps(__m128 __a) {
  return __builtin_ia32_movmskps((__v4sf)__a);
}
# 3172 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h"
# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h" 1
# 3173 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/xmmintrin.h" 2
# 18 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h" 2

typedef double __m128d __attribute__((__vector_size__(16), __aligned__(16)));

typedef double __m128d_u __attribute__((__vector_size__(16), __aligned__(1)));
typedef long long __m128i_u
    __attribute__((__vector_size__(16), __aligned__(1)));


typedef double __v2df __attribute__((__vector_size__(16)));


typedef unsigned long long __v2du __attribute__((__vector_size__(16)));



typedef signed char __v16qs __attribute__((__vector_size__(16)));



typedef _Float16 __v8hf __attribute__((__vector_size__(16), __aligned__(16)));
typedef _Float16 __m128h __attribute__((__vector_size__(16), __aligned__(16)));
typedef _Float16 __m128h_u __attribute__((__vector_size__(16), __aligned__(1)));

typedef __bf16 __v8bf __attribute__((__vector_size__(16), __aligned__(16)));
typedef __bf16 __m128bh __attribute__((__vector_size__(16), __aligned__(16)));
# 80 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_add_sd(__m128d __a,
                                                                  __m128d __b) {
  __a[0] += __b[0];
  return __a;
}
# 98 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_add_pd(__m128d __a,
                                                                  __m128d __b) {
  return (__m128d)((__v2df)__a + (__v2df)__b);
}
# 120 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_sub_sd(__m128d __a,
                                                                  __m128d __b) {
  __a[0] -= __b[0];
  return __a;
}
# 138 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_sub_pd(__m128d __a,
                                                                  __m128d __b) {
  return (__m128d)((__v2df)__a - (__v2df)__b);
}
# 159 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_mul_sd(__m128d __a,
                                                                  __m128d __b) {
  __a[0] *= __b[0];
  return __a;
}
# 177 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_mul_pd(__m128d __a,
                                                                  __m128d __b) {
  return (__m128d)((__v2df)__a * (__v2df)__b);
}
# 199 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_div_sd(__m128d __a,
                                                                  __m128d __b) {
  __a[0] /= __b[0];
  return __a;
}
# 218 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_div_pd(__m128d __a,
                                                                  __m128d __b) {
  return (__m128d)((__v2df)__a / (__v2df)__b);
}
# 242 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_sqrt_sd(__m128d __a,
                                                         __m128d __b) {
  return __extension__(__m128d){__builtin_elementwise_sqrt(__b[0]), __a[1]};
}
# 258 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_sqrt_pd(__m128d __a) {
  return __builtin_elementwise_sqrt(__a);
}
# 282 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_min_sd(__m128d __a,
                                                        __m128d __b) {
  return __builtin_ia32_minsd((__v2df)__a, (__v2df)__b);
}
# 303 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_min_pd(__m128d __a,
                                                                  __m128d __b) {
  return __builtin_ia32_minpd((__v2df)__a, (__v2df)__b);
}
# 328 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_max_sd(__m128d __a,
                                                        __m128d __b) {
  return __builtin_ia32_maxsd((__v2df)__a, (__v2df)__b);
}
# 349 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_max_pd(__m128d __a,
                                                                  __m128d __b) {
  return __builtin_ia32_maxpd((__v2df)__a, (__v2df)__b);
}
# 366 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_and_pd(__m128d __a,
                                                                  __m128d __b) {
  return (__m128d)((__v2du)__a & (__v2du)__b);
}
# 386 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_andnot_pd(__m128d __a, __m128d __b) {
  return (__m128d)(~(__v2du)__a & (__v2du)__b);
}
# 403 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_or_pd(__m128d __a,
                                                                 __m128d __b) {
  return (__m128d)((__v2du)__a | (__v2du)__b);
}
# 420 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_xor_pd(__m128d __a,
                                                                  __m128d __b) {
  return (__m128d)((__v2du)__a ^ (__v2du)__b);
}
# 440 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cmpeq_pd(__m128d __a,
                                                          __m128d __b) {
  return (__m128d)__builtin_ia32_cmpeqpd((__v2df)__a, (__v2df)__b);
}
# 461 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cmplt_pd(__m128d __a,
                                                          __m128d __b) {
  return (__m128d)__builtin_ia32_cmpltpd((__v2df)__a, (__v2df)__b);
}
# 482 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cmple_pd(__m128d __a,
                                                          __m128d __b) {
  return (__m128d)__builtin_ia32_cmplepd((__v2df)__a, (__v2df)__b);
}
# 503 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cmpgt_pd(__m128d __a,
                                                          __m128d __b) {
  return (__m128d)__builtin_ia32_cmpltpd((__v2df)__b, (__v2df)__a);
}
# 524 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cmpge_pd(__m128d __a,
                                                          __m128d __b) {
  return (__m128d)__builtin_ia32_cmplepd((__v2df)__b, (__v2df)__a);
}
# 546 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cmpord_pd(__m128d __a,
                                                           __m128d __b) {
  return (__m128d)__builtin_ia32_cmpordpd((__v2df)__a, (__v2df)__b);
}
# 569 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cmpunord_pd(__m128d __a,
                                                             __m128d __b) {
  return (__m128d)__builtin_ia32_cmpunordpd((__v2df)__a, (__v2df)__b);
}
# 590 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cmpneq_pd(__m128d __a,
                                                           __m128d __b) {
  return (__m128d)__builtin_ia32_cmpneqpd((__v2df)__a, (__v2df)__b);
}
# 611 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cmpnlt_pd(__m128d __a,
                                                           __m128d __b) {
  return (__m128d)__builtin_ia32_cmpnltpd((__v2df)__a, (__v2df)__b);
}
# 632 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cmpnle_pd(__m128d __a,
                                                           __m128d __b) {
  return (__m128d)__builtin_ia32_cmpnlepd((__v2df)__a, (__v2df)__b);
}
# 653 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cmpngt_pd(__m128d __a,
                                                           __m128d __b) {
  return (__m128d)__builtin_ia32_cmpnltpd((__v2df)__b, (__v2df)__a);
}
# 674 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cmpnge_pd(__m128d __a,
                                                           __m128d __b) {
  return (__m128d)__builtin_ia32_cmpnlepd((__v2df)__b, (__v2df)__a);
}
# 697 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cmpeq_sd(__m128d __a,
                                                          __m128d __b) {
  return (__m128d)__builtin_ia32_cmpeqsd((__v2df)__a, (__v2df)__b);
}
# 722 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cmplt_sd(__m128d __a,
                                                          __m128d __b) {
  return (__m128d)__builtin_ia32_cmpltsd((__v2df)__a, (__v2df)__b);
}
# 747 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cmple_sd(__m128d __a,
                                                          __m128d __b) {
  return (__m128d)__builtin_ia32_cmplesd((__v2df)__a, (__v2df)__b);
}
# 772 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cmpgt_sd(__m128d __a,
                                                          __m128d __b) {
  __m128d __c = __builtin_ia32_cmpltsd((__v2df)__b, (__v2df)__a);
  return __extension__(__m128d){__c[0], __a[1]};
}
# 798 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cmpge_sd(__m128d __a,
                                                          __m128d __b) {
  __m128d __c = __builtin_ia32_cmplesd((__v2df)__b, (__v2df)__a);
  return __extension__(__m128d){__c[0], __a[1]};
}
# 825 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cmpord_sd(__m128d __a,
                                                           __m128d __b) {
  return (__m128d)__builtin_ia32_cmpordsd((__v2df)__a, (__v2df)__b);
}
# 852 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cmpunord_sd(__m128d __a,
                                                             __m128d __b) {
  return (__m128d)__builtin_ia32_cmpunordsd((__v2df)__a, (__v2df)__b);
}
# 877 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cmpneq_sd(__m128d __a,
                                                           __m128d __b) {
  return (__m128d)__builtin_ia32_cmpneqsd((__v2df)__a, (__v2df)__b);
}
# 902 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cmpnlt_sd(__m128d __a,
                                                           __m128d __b) {
  return (__m128d)__builtin_ia32_cmpnltsd((__v2df)__a, (__v2df)__b);
}
# 927 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cmpnle_sd(__m128d __a,
                                                           __m128d __b) {
  return (__m128d)__builtin_ia32_cmpnlesd((__v2df)__a, (__v2df)__b);
}
# 952 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cmpngt_sd(__m128d __a,
                                                           __m128d __b) {
  __m128d __c = __builtin_ia32_cmpnltsd((__v2df)__b, (__v2df)__a);
  return __extension__(__m128d){__c[0], __a[1]};
}
# 978 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cmpnge_sd(__m128d __a,
                                                           __m128d __b) {
  __m128d __c = __builtin_ia32_cmpnlesd((__v2df)__b, (__v2df)__a);
  return __extension__(__m128d){__c[0], __a[1]};
}
# 1001 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ int __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_comieq_sd(__m128d __a,
                                                       __m128d __b) {
  return __builtin_ia32_comisdeq((__v2df)__a, (__v2df)__b);
}
# 1025 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ int __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_comilt_sd(__m128d __a,
                                                       __m128d __b) {
  return __builtin_ia32_comisdlt((__v2df)__a, (__v2df)__b);
}
# 1049 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ int __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_comile_sd(__m128d __a,
                                                       __m128d __b) {
  return __builtin_ia32_comisdle((__v2df)__a, (__v2df)__b);
}
# 1073 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ int __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_comigt_sd(__m128d __a,
                                                       __m128d __b) {
  return __builtin_ia32_comisdgt((__v2df)__a, (__v2df)__b);
}
# 1097 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ int __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_comige_sd(__m128d __a,
                                                       __m128d __b) {
  return __builtin_ia32_comisdge((__v2df)__a, (__v2df)__b);
}
# 1121 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ int __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_comineq_sd(__m128d __a,
                                                        __m128d __b) {
  return __builtin_ia32_comisdneq((__v2df)__a, (__v2df)__b);
}
# 1143 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ int __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_ucomieq_sd(__m128d __a,
                                                        __m128d __b) {
  return __builtin_ia32_ucomisdeq((__v2df)__a, (__v2df)__b);
}
# 1167 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ int __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_ucomilt_sd(__m128d __a,
                                                        __m128d __b) {
  return __builtin_ia32_ucomisdlt((__v2df)__a, (__v2df)__b);
}
# 1191 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ int __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_ucomile_sd(__m128d __a,
                                                        __m128d __b) {
  return __builtin_ia32_ucomisdle((__v2df)__a, (__v2df)__b);
}
# 1215 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ int __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_ucomigt_sd(__m128d __a,
                                                        __m128d __b) {
  return __builtin_ia32_ucomisdgt((__v2df)__a, (__v2df)__b);
}
# 1239 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ int __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_ucomige_sd(__m128d __a,
                                                        __m128d __b) {
  return __builtin_ia32_ucomisdge((__v2df)__a, (__v2df)__b);
}
# 1263 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ int __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_ucomineq_sd(__m128d __a,
                                                         __m128d __b) {
  return __builtin_ia32_ucomisdneq((__v2df)__a, (__v2df)__b);
}
# 1281 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_cvtpd_ps(__m128d __a) {
  return __builtin_ia32_cvtpd2ps((__v2df)__a);
}
# 1300 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_cvtps_pd(__m128 __a) {
  return (__m128d) __builtin_convertvector(
      __builtin_shufflevector((__v4sf)__a, (__v4sf)__a, 0, 1), __v2df);
}
# 1322 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_cvtepi32_pd(__m128i __a) {
  return (__m128d) __builtin_convertvector(
      __builtin_shufflevector((__v4si)__a, (__v4si)__a, 0, 1), __v2df);
}
# 1345 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cvtpd_epi32(__m128d __a) {
  return __builtin_ia32_cvtpd2dq((__v2df)__a);
}
# 1364 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ int __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cvtsd_si32(__m128d __a) {
  return __builtin_ia32_cvtsd2si((__v2df)__a);
}
# 1387 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_cvtsd_ss(__m128 __a, __m128d __b) {
  return (__m128)__builtin_ia32_cvtsd2ss((__v4sf)__a, (__v2df)__b);
}
# 1409 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_cvtsi32_sd(__m128d __a, int __b) {
  __a[0] = __b;
  return __a;
}
# 1434 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_cvtss_sd(__m128d __a, __m128 __b) {
  __a[0] = __b[0];
  return __a;
}
# 1458 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cvttpd_epi32(__m128d __a) {
  return (__m128i)__builtin_ia32_cvttpd2dq((__v2df)__a);
}
# 1478 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ int __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cvttsd_si32(__m128d __a) {
  return __builtin_ia32_cvttsd2si((__v2df)__a);
}
# 1497 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cvtpd_pi32(__m128d __a) {
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_cvtpd2dq((__v2df)__a)), __extension__(__v2di){}, 0);
}
# 1516 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cvttpd_pi32(__m128d __a) {
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_cvttpd2dq((__v2df)__a)), __extension__(__v2di){}, 0);
}
# 1531 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_cvtpi32_pd(__m64 __a) {
  return (__m128d) __builtin_convertvector((__v2si)__a, __v2df);
}
# 1547 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ double __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_cvtsd_f64(__m128d __a) {
  return __a[0];
}
# 1563 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_load_pd(double const *__dp) {
  return *(const __m128d *)__dp;
}
# 1579 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_load1_pd(double const *__dp) {
  struct __mm_load1_pd_struct {
    double __u;
  } __attribute__((__packed__, __may_alias__));
  double __u = ((const struct __mm_load1_pd_struct *)__dp)->__u;
  return __extension__(__m128d){__u, __u};
}
# 1603 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_loadr_pd(double const *__dp) {
  __m128d __u = *(const __m128d *)__dp;
  return __builtin_shufflevector((__v2df)__u, (__v2df)__u, 1, 0);
}
# 1619 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_loadu_pd(double const *__dp) {
  struct __loadu_pd {
    __m128d_u __v;
  } __attribute__((__packed__, __may_alias__));
  return ((const struct __loadu_pd *)__dp)->__v;
}
# 1637 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_loadu_si64(void const *__a) {
  struct __loadu_si64 {
    long long __v;
  } __attribute__((__packed__, __may_alias__));
  long long __u = ((const struct __loadu_si64 *)__a)->__v;
  return __extension__(__m128i)(__v2di){__u, 0LL};
}
# 1656 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_loadu_si32(void const *__a) {
  struct __loadu_si32 {
    int __v;
  } __attribute__((__packed__, __may_alias__));
  int __u = ((const struct __loadu_si32 *)__a)->__v;
  return __extension__(__m128i)(__v4si){__u, 0, 0, 0};
}
# 1675 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_loadu_si16(void const *__a) {
  struct __loadu_si16 {
    short __v;
  } __attribute__((__packed__, __may_alias__));
  short __u = ((const struct __loadu_si16 *)__a)->__v;
  return __extension__(__m128i)(__v8hi){__u, 0, 0, 0, 0, 0, 0, 0};
}
# 1694 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_load_sd(double const *__dp) {
  struct __mm_load_sd_struct {
    double __u;
  } __attribute__((__packed__, __may_alias__));
  double __u = ((const struct __mm_load_sd_struct *)__dp)->__u;
  return __extension__(__m128d){__u, 0};
}
# 1719 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_loadh_pd(__m128d __a,
                                                          double const *__dp) {
  struct __mm_loadh_pd_struct {
    double __u;
  } __attribute__((__packed__, __may_alias__));
  double __u = ((const struct __mm_loadh_pd_struct *)__dp)->__u;
  return __extension__(__m128d){__a[0], __u};
}
# 1745 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_loadl_pd(__m128d __a,
                                                          double const *__dp) {
  struct __mm_loadl_pd_struct {
    double __u;
  } __attribute__((__packed__, __may_alias__));
  double __u = ((const struct __mm_loadl_pd_struct *)__dp)->__u;
  return __extension__(__m128d){__u, __a[1]};
}
# 1765 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_undefined_pd(void) {
  return (__m128d)__builtin_ia32_undef128();
}
# 1783 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_set_sd(double __w) {
  return __extension__(__m128d){__w, 0.0};
}
# 1799 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_set1_pd(double __w) {
  return __extension__(__m128d){__w, __w};
}
# 1815 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_set_pd1(double __w) {
  return _mm_set1_pd(__w);
}
# 1833 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_set_pd(double __w,
                                                                  double __x) {
  return __extension__(__m128d){__x, __w};
}
# 1853 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_setr_pd(double __w,
                                                                   double __x) {
  return __extension__(__m128d){__w, __x};
}
# 1867 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_setzero_pd(void) {
  return __extension__(__m128d){0.0, 0.0};
}
# 1886 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_move_sd(__m128d __a, __m128d __b) {
  __a[0] = __b[0];
  return __a;
}
# 1903 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ void __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_store_sd(double *__dp,
                                                       __m128d __a) {
  struct __mm_store_sd_struct {
    double __u;
  } __attribute__((__packed__, __may_alias__));
  ((struct __mm_store_sd_struct *)__dp)->__u = __a[0];
}
# 1924 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ void __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_store_pd(double *__dp,
                                                       __m128d __a) {
  *(__m128d *)__dp = __a;
}
# 1943 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ void __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_store1_pd(double *__dp,
                                                        __m128d __a) {
  __a = __builtin_shufflevector((__v2df)__a, (__v2df)__a, 0, 0);
  _mm_store_pd(__dp, __a);
}
# 1963 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ void __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_store_pd1(double *__dp,
                                                        __m128d __a) {
  _mm_store1_pd(__dp, __a);
}
# 1980 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ void __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_storeu_pd(double *__dp,
                                                        __m128d __a) {
  struct __storeu_pd {
    __m128d_u __v;
  } __attribute__((__packed__, __may_alias__));
  ((struct __storeu_pd *)__dp)->__v = __a;
}
# 2002 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ void __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_storer_pd(double *__dp,
                                                        __m128d __a) {
  __a = __builtin_shufflevector((__v2df)__a, (__v2df)__a, 1, 0);
  *(__m128d *)__dp = __a;
}
# 2019 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ void __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_storeh_pd(double *__dp,
                                                        __m128d __a) {
  struct __mm_storeh_pd_struct {
    double __u;
  } __attribute__((__packed__, __may_alias__));
  ((struct __mm_storeh_pd_struct *)__dp)->__u = __a[1];
}
# 2038 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ void __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_storel_pd(double *__dp,
                                                        __m128d __a) {
  struct __mm_storeh_pd_struct {
    double __u;
  } __attribute__((__packed__, __may_alias__));
  ((struct __mm_storeh_pd_struct *)__dp)->__u = __a[0];
}
# 2062 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_add_epi8(__m128i __a, __m128i __b) {
  return (__m128i)((__v16qu)__a + (__v16qu)__b);
}
# 2083 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_add_epi16(__m128i __a, __m128i __b) {
  return (__m128i)((__v8hu)__a + (__v8hu)__b);
}
# 2104 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_add_epi32(__m128i __a, __m128i __b) {
  return (__m128i)((__v4su)__a + (__v4su)__b);
}
# 2121 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_add_si64(__m64 __a,
                                                                  __m64 __b) {
  return (__m64)(((__v1du)__a)[0] + ((__v1du)__b)[0]);
}
# 2142 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_add_epi64(__m128i __a, __m128i __b) {
  return (__m128i)((__v2du)__a + (__v2du)__b);
}
# 2164 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_adds_epi8(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_elementwise_add_sat((__v16qs)__a, (__v16qs)__b);
}
# 2186 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_adds_epi16(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_elementwise_add_sat((__v8hi)__a, (__v8hi)__b);
}
# 2208 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_adds_epu8(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_elementwise_add_sat((__v16qu)__a, (__v16qu)__b);
}
# 2230 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_adds_epu16(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_elementwise_add_sat((__v8hu)__a, (__v8hu)__b);
}
# 2249 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_avg_epu8(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_ia32_pavgb128((__v16qu)__a, (__v16qu)__b);
}
# 2268 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_avg_epu16(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_ia32_pavgw128((__v8hu)__a, (__v8hu)__b);
}
# 2293 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_madd_epi16(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_ia32_pmaddwd128((__v8hi)__a, (__v8hi)__b);
}
# 2312 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_max_epi16(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_elementwise_max((__v8hi)__a, (__v8hi)__b);
}
# 2331 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_max_epu8(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_elementwise_max((__v16qu)__a, (__v16qu)__b);
}
# 2350 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_min_epi16(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_elementwise_min((__v8hi)__a, (__v8hi)__b);
}
# 2369 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_min_epu8(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_elementwise_min((__v16qu)__a, (__v16qu)__b);
}
# 2388 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_mulhi_epi16(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_ia32_pmulhw128((__v8hi)__a, (__v8hi)__b);
}
# 2407 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_mulhi_epu16(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_ia32_pmulhuw128((__v8hu)__a, (__v8hu)__b);
}
# 2426 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_mullo_epi16(__m128i __a, __m128i __b) {
  return (__m128i)((__v8hu)__a * (__v8hu)__b);
}
# 2444 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_mul_su32(__m64 __a,
                                                                  __m64 __b) {
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_pmuludq128((__v4si)(__m128i) __builtin_shufflevector((__v2si)(__a), __extension__(__v2si){}, 0, 1, 2, 3), (__v4si)(__m128i) __builtin_shufflevector((__v2si)(__b), __extension__(__v2si){}, 0, 1, 2, 3))), __extension__(__v2di){}, 0);

}
# 2463 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_mul_epu32(__m128i __a, __m128i __b) {
  return __builtin_ia32_pmuludq128((__v4si)__a, (__v4si)__b);
}
# 2484 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_sad_epu8(__m128i __a,
                                                          __m128i __b) {
  return __builtin_ia32_psadbw128((__v16qi)__a, (__v16qi)__b);
}
# 2501 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_sub_epi8(__m128i __a, __m128i __b) {
  return (__m128i)((__v16qu)__a - (__v16qu)__b);
}
# 2518 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_sub_epi16(__m128i __a, __m128i __b) {
  return (__m128i)((__v8hu)__a - (__v8hu)__b);
}
# 2535 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_sub_epi32(__m128i __a, __m128i __b) {
  return (__m128i)((__v4su)__a - (__v4su)__b);
}
# 2553 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_sub_si64(__m64 __a,
                                                                  __m64 __b) {
  return (__m64)(((__v1du)__a)[0] - ((__v1du)__b)[0]);
}
# 2570 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_sub_epi64(__m128i __a, __m128i __b) {
  return (__m128i)((__v2du)__a - (__v2du)__b);
}
# 2592 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_subs_epi8(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_elementwise_sub_sat((__v16qs)__a, (__v16qs)__b);
}
# 2614 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_subs_epi16(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_elementwise_sub_sat((__v8hi)__a, (__v8hi)__b);
}
# 2635 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_subs_epu8(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_elementwise_sub_sat((__v16qu)__a, (__v16qu)__b);
}
# 2656 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_subs_epu16(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_elementwise_sub_sat((__v8hu)__a, (__v8hu)__b);
}
# 2673 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_and_si128(__m128i __a, __m128i __b) {
  return (__m128i)((__v2du)__a & (__v2du)__b);
}
# 2692 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_andnot_si128(__m128i __a, __m128i __b) {
  return (__m128i)(~(__v2du)__a & (__v2du)__b);
}
# 2708 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_or_si128(__m128i __a, __m128i __b) {
  return (__m128i)((__v2du)__a | (__v2du)__b);
}
# 2725 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_xor_si128(__m128i __a, __m128i __b) {
  return (__m128i)((__v2du)__a ^ (__v2du)__b);
}
# 2768 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_slli_epi16(__m128i __a, int __count) {
  return (__m128i)__builtin_ia32_psllwi128((__v8hi)__a, __count);
}
# 2786 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_sll_epi16(__m128i __a, __m128i __count) {
  return (__m128i)__builtin_ia32_psllw128((__v8hi)__a, (__v8hi)__count);
}
# 2804 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_slli_epi32(__m128i __a, int __count) {
  return (__m128i)__builtin_ia32_pslldi128((__v4si)__a, __count);
}
# 2822 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_sll_epi32(__m128i __a, __m128i __count) {
  return (__m128i)__builtin_ia32_pslld128((__v4si)__a, (__v4si)__count);
}
# 2840 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_slli_epi64(__m128i __a, int __count) {
  return __builtin_ia32_psllqi128((__v2di)__a, __count);
}
# 2858 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_sll_epi64(__m128i __a, __m128i __count) {
  return __builtin_ia32_psllq128((__v2di)__a, (__v2di)__count);
}
# 2877 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_srai_epi16(__m128i __a, int __count) {
  return (__m128i)__builtin_ia32_psrawi128((__v8hi)__a, __count);
}
# 2896 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_sra_epi16(__m128i __a, __m128i __count) {
  return (__m128i)__builtin_ia32_psraw128((__v8hi)__a, (__v8hi)__count);
}
# 2915 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_srai_epi32(__m128i __a, int __count) {
  return (__m128i)__builtin_ia32_psradi128((__v4si)__a, __count);
}
# 2934 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_sra_epi32(__m128i __a, __m128i __count) {
  return (__m128i)__builtin_ia32_psrad128((__v4si)__a, (__v4si)__count);
}
# 2977 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_srli_epi16(__m128i __a, int __count) {
  return (__m128i)__builtin_ia32_psrlwi128((__v8hi)__a, __count);
}
# 2995 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_srl_epi16(__m128i __a, __m128i __count) {
  return (__m128i)__builtin_ia32_psrlw128((__v8hi)__a, (__v8hi)__count);
}
# 3013 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_srli_epi32(__m128i __a, int __count) {
  return (__m128i)__builtin_ia32_psrldi128((__v4si)__a, __count);
}
# 3031 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_srl_epi32(__m128i __a, __m128i __count) {
  return (__m128i)__builtin_ia32_psrld128((__v4si)__a, (__v4si)__count);
}
# 3049 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_srli_epi64(__m128i __a, int __count) {
  return __builtin_ia32_psrlqi128((__v2di)__a, __count);
}
# 3067 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_srl_epi64(__m128i __a, __m128i __count) {
  return __builtin_ia32_psrlq128((__v2di)__a, (__v2di)__count);
}
# 3086 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_cmpeq_epi8(__m128i __a, __m128i __b) {
  return (__m128i)((__v16qi)__a == (__v16qi)__b);
}
# 3105 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_cmpeq_epi16(__m128i __a, __m128i __b) {
  return (__m128i)((__v8hi)__a == (__v8hi)__b);
}
# 3124 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_cmpeq_epi32(__m128i __a, __m128i __b) {
  return (__m128i)((__v4si)__a == (__v4si)__b);
}
# 3144 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_cmpgt_epi8(__m128i __a, __m128i __b) {


  return (__m128i)((__v16qs)__a > (__v16qs)__b);
}
# 3166 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_cmpgt_epi16(__m128i __a, __m128i __b) {
  return (__m128i)((__v8hi)__a > (__v8hi)__b);
}
# 3186 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_cmpgt_epi32(__m128i __a, __m128i __b) {
  return (__m128i)((__v4si)__a > (__v4si)__b);
}
# 3206 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_cmplt_epi8(__m128i __a, __m128i __b) {
  return _mm_cmpgt_epi8(__b, __a);
}
# 3226 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_cmplt_epi16(__m128i __a, __m128i __b) {
  return _mm_cmpgt_epi16(__b, __a);
}
# 3246 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_cmplt_epi32(__m128i __a, __m128i __b) {
  return _mm_cmpgt_epi32(__b, __a);
}
# 3269 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_cvtsi64_sd(__m128d __a, long long __b) {
  __a[0] = __b;
  return __a;
}
# 3290 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ long long __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cvtsd_si64(__m128d __a) {
  return __builtin_ia32_cvtsd2si64((__v2df)__a);
}
# 3310 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ long long __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cvttsd_si64(__m128d __a) {
  return __builtin_ia32_cvttsd2si64((__v2df)__a);
}
# 3324 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_cvtepi32_ps(__m128i __a) {
  return (__m128) __builtin_convertvector((__v4si)__a, __v4sf);
}
# 3343 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cvtps_epi32(__m128 __a) {
  return (__m128i)__builtin_ia32_cvtps2dq((__v4sf)__a);
}
# 3362 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_cvttps_epi32(__m128 __a) {
  return (__m128i)__builtin_ia32_cvttps2dq((__v4sf)__a);
}
# 3376 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_cvtsi32_si128(int __a) {
  return __extension__(__m128i)(__v4si){__a, 0, 0, 0};
}
# 3392 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_cvtsi64_si128(long long __a) {
  return __extension__(__m128i)(__v2di){__a, 0};
}
# 3408 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ int __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_cvtsi128_si32(__m128i __a) {
  __v4si __b = (__v4si)__a;
  return __b[0];
}
# 3425 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ long long __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_cvtsi128_si64(__m128i __a) {
  return __a[0];
}
# 3440 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_load_si128(__m128i const *__p) {
  return *__p;
}
# 3455 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_loadu_si128(__m128i_u const *__p) {
  struct __loadu_si128 {
    __m128i_u __v;
  } __attribute__((__packed__, __may_alias__));
  return ((const struct __loadu_si128 *)__p)->__v;
}
# 3475 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_loadl_epi64(__m128i_u const *__p) {
  struct __mm_loadl_epi64_struct {
    long long __u;
  } __attribute__((__packed__, __may_alias__));
  return __extension__(__m128i){
      ((const struct __mm_loadl_epi64_struct *)__p)->__u, 0};
}
# 3493 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_undefined_si128(void) {
  return (__m128i)__builtin_ia32_undef128();
}
# 3513 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_set_epi64x(long long __q1, long long __q0) {
  return __extension__(__m128i)(__v2di){__q0, __q1};
}
# 3534 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_set_epi64(__m64 __q1, __m64 __q0) {
  return _mm_set_epi64x((long long)__q1[0], (long long)__q0[0]);
}
# 3561 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_set_epi32(int __i3,
                                                                     int __i2,
                                                                     int __i1,
                                                                     int __i0) {
  return __extension__(__m128i)(__v4si){__i0, __i1, __i2, __i3};
}
# 3602 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_set_epi16(short __w7, short __w6, short __w5, short __w4, short __w3,
              short __w2, short __w1, short __w0) {
  return __extension__(__m128i)(__v8hi){__w0, __w1, __w2, __w3,
                                        __w4, __w5, __w6, __w7};
}
# 3651 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_set_epi8(char __b15, char __b14, char __b13, char __b12, char __b11,
             char __b10, char __b9, char __b8, char __b7, char __b6, char __b5,
             char __b4, char __b3, char __b2, char __b1, char __b0) {
  return __extension__(__m128i)(__v16qi){
      __b0, __b1, __b2, __b3, __b4, __b5, __b6, __b7,
      __b8, __b9, __b10, __b11, __b12, __b13, __b14, __b15};
}
# 3673 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_set1_epi64x(long long __q) {
  return _mm_set_epi64x(__q, __q);
}
# 3691 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_set1_epi64(__m64 __q) {
  return _mm_set_epi64(__q, __q);
}
# 3709 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_set1_epi32(int __i) {
  return _mm_set_epi32(__i, __i, __i, __i);
}
# 3726 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_set1_epi16(short __w) {
  return _mm_set_epi16(__w, __w, __w, __w, __w, __w, __w, __w);
}
# 3744 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_set1_epi8(char __b) {
  return _mm_set_epi8(__b, __b, __b, __b, __b, __b, __b, __b, __b, __b, __b,
                      __b, __b, __b, __b, __b);
}
# 3763 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_setr_epi64(__m64 __q0, __m64 __q1) {
  return _mm_set_epi64(__q1, __q0);
}
# 3785 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_setr_epi32(int __i0, int __i1, int __i2, int __i3) {
  return _mm_set_epi32(__i3, __i2, __i1, __i0);
}
# 3815 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_setr_epi16(short __w0, short __w1, short __w2, short __w3, short __w4,
               short __w5, short __w6, short __w7) {
  return _mm_set_epi16(__w7, __w6, __w5, __w4, __w3, __w2, __w1, __w0);
}
# 3862 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_setr_epi8(char __b0, char __b1, char __b2, char __b3, char __b4, char __b5,
              char __b6, char __b7, char __b8, char __b9, char __b10,
              char __b11, char __b12, char __b13, char __b14, char __b15) {
  return _mm_set_epi8(__b15, __b14, __b13, __b12, __b11, __b10, __b9, __b8,
                      __b7, __b6, __b5, __b4, __b3, __b2, __b1, __b0);
}
# 3878 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_setzero_si128(void) {
  return __extension__(__m128i)(__v2di){0LL, 0LL};
}
# 3894 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ void __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_store_si128(__m128i *__p,
                                                          __m128i __b) {
  *__p = __b;
}
# 3909 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ void __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_storeu_si128(__m128i_u *__p,
                                                           __m128i __b) {
  struct __storeu_si128 {
    __m128i_u __v;
  } __attribute__((__packed__, __may_alias__));
  ((struct __storeu_si128 *)__p)->__v = __b;
}
# 3929 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ void __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_storeu_si64(void *__p,
                                                          __m128i __b) {
  struct __storeu_si64 {
    long long __v;
  } __attribute__((__packed__, __may_alias__));
  ((struct __storeu_si64 *)__p)->__v = ((__v2di)__b)[0];
}
# 3949 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ void __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_storeu_si32(void *__p,
                                                          __m128i __b) {
  struct __storeu_si32 {
    int __v;
  } __attribute__((__packed__, __may_alias__));
  ((struct __storeu_si32 *)__p)->__v = ((__v4si)__b)[0];
}
# 3969 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ void __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_storeu_si16(void *__p,
                                                          __m128i __b) {
  struct __storeu_si16 {
    short __v;
  } __attribute__((__packed__, __may_alias__));
  ((struct __storeu_si16 *)__p)->__v = ((__v8hi)__b)[0];
}
# 3998 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ void __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_maskmoveu_si128(__m128i __d,
                                                              __m128i __n,
                                                              char *__p) {
  __builtin_ia32_maskmovdqu((__v16qi)__d, (__v16qi)__n, __p);
}
# 4017 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ void __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_storel_epi64(__m128i_u *__p,
                                                           __m128i __a) {
  struct __mm_storel_epi64_struct {
    long long __u;
  } __attribute__((__packed__, __may_alias__));
  ((struct __mm_storel_epi64_struct *)__p)->__u = __a[0];
}
# 4039 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ void __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_stream_pd(void *__p,
                                                        __m128d __a) {
  __builtin_nontemporal_store((__v2df)__a, (__v2df *)__p);
}
# 4057 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ void __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128))) _mm_stream_si128(void *__p,
                                                           __m128i __a) {
  __builtin_nontemporal_store((__v2di)__a, (__v2di *)__p);
}
# 4075 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ void
    __attribute__((__always_inline__, __nodebug__, __target__("sse2")))
    _mm_stream_si32(void *__p, int __a) {
  __builtin_ia32_movnti((int *)__p, __a);
}
# 4095 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ void
    __attribute__((__always_inline__, __nodebug__, __target__("sse2")))
    _mm_stream_si64(void *__p, long long __a) {
  __builtin_ia32_movnti64((long long *)__p, __a);
}
# 4116 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
void _mm_clflush(void const *__p);
# 4127 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
void _mm_lfence(void);
# 4138 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
void _mm_mfence(void);
# 4162 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_packs_epi16(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_ia32_packsswb128((__v8hi)__a, (__v8hi)__b);
}
# 4185 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_packs_epi32(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_ia32_packssdw128((__v4si)__a, (__v4si)__b);
}
# 4208 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_packus_epi16(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_ia32_packuswb128((__v8hi)__a, (__v8hi)__b);
}
# 4283 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ int __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_movemask_epi8(__m128i __a) {
  return __builtin_ia32_pmovmskb128((__v16qi)__a);
}
# 4417 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_unpackhi_epi8(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_shufflevector(
      (__v16qi)__a, (__v16qi)__b, 8, 16 + 8, 9, 16 + 9, 10, 16 + 10, 11,
      16 + 11, 12, 16 + 12, 13, 16 + 13, 14, 16 + 14, 15, 16 + 15);
}
# 4445 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_unpackhi_epi16(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_shufflevector((__v8hi)__a, (__v8hi)__b, 4, 8 + 4, 5,
                                          8 + 5, 6, 8 + 6, 7, 8 + 7);
}
# 4468 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_unpackhi_epi32(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_shufflevector((__v4si)__a, (__v4si)__b, 2, 4 + 2, 3,
                                          4 + 3);
}
# 4489 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_unpackhi_epi64(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_shufflevector((__v2di)__a, (__v2di)__b, 1, 2 + 1);
}
# 4523 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_unpacklo_epi8(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_shufflevector(
      (__v16qi)__a, (__v16qi)__b, 0, 16 + 0, 1, 16 + 1, 2, 16 + 2, 3, 16 + 3, 4,
      16 + 4, 5, 16 + 5, 6, 16 + 6, 7, 16 + 7);
}
# 4552 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_unpacklo_epi16(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_shufflevector((__v8hi)__a, (__v8hi)__b, 0, 8 + 0, 1,
                                          8 + 1, 2, 8 + 2, 3, 8 + 3);
}
# 4575 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_unpacklo_epi32(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_shufflevector((__v4si)__a, (__v4si)__b, 0, 4 + 0, 1,
                                          4 + 1);
}
# 4596 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_unpacklo_epi64(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_shufflevector((__v2di)__a, (__v2di)__b, 0, 2 + 0);
}
# 4612 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_movepi64_pi64(__m128i __a) {
  return (__m64)__a[0];
}
# 4628 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_movpi64_epi64(__m64 __a) {
  return __builtin_shufflevector((__v1di)__a, _mm_setzero_si64(), 0, 1);
}
# 4645 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_move_epi64(__m128i __a) {
  return __builtin_shufflevector((__v2di)__a, _mm_setzero_si128(), 0, 2);
}
# 4665 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_unpackhi_pd(__m128d __a, __m128d __b) {
  return __builtin_shufflevector((__v2df)__a, (__v2df)__b, 1, 2 + 1);
}
# 4685 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_unpacklo_pd(__m128d __a, __m128d __b) {
  return __builtin_shufflevector((__v2df)__a, (__v2df)__b, 0, 2 + 0);
}
# 4703 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ int __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_movemask_pd(__m128d __a) {
  return __builtin_ia32_movmskpd((__v2df)__a);
}
# 4750 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_castpd_ps(__m128d __a) {
  return (__m128)__a;
}
# 4766 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_castpd_si128(__m128d __a) {
  return (__m128i)__a;
}
# 4782 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_castps_pd(__m128 __a) {
  return (__m128d)__a;
}
# 4798 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_castps_si128(__m128 __a) {
  return (__m128i)__a;
}
# 4814 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_castsi128_ps(__m128i __a) {
  return (__m128)__a;
}
# 4830 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse2"), __min_vector_width__(128)))
_mm_castsi128_pd(__m128i __a) {
  return (__m128d)__a;
}
# 4918 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/emmintrin.h"
void _mm_pause(void);
# 18 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/pmmintrin.h" 2
# 44 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/pmmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("sse3"), __min_vector_width__(128)))
_mm_lddqu_si128(__m128i_u const *__p)
{
  return (__m128i)__builtin_ia32_lddqu((char const *)__p);
}
# 63 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/pmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse3"), __min_vector_width__(128)))
_mm_addsub_ps(__m128 __a, __m128 __b) {
  return __builtin_ia32_addsubps((__v4sf)__a, (__v4sf)__b);
}
# 85 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/pmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse3"), __min_vector_width__(128))) _mm_hadd_ps(__m128 __a,
                                                                  __m128 __b) {
  return __builtin_ia32_haddps((__v4sf)__a, (__v4sf)__b);
}
# 107 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/pmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse3"), __min_vector_width__(128))) _mm_hsub_ps(__m128 __a,
                                                                  __m128 __b) {
  return __builtin_ia32_hsubps((__v4sf)__a, (__v4sf)__b);
}
# 128 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/pmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse3"), __min_vector_width__(128)))
_mm_movehdup_ps(__m128 __a)
{
  return __builtin_shufflevector((__v4sf)__a, (__v4sf)__a, 1, 1, 3, 3);
}
# 149 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/pmmintrin.h"
static __inline__ __m128 __attribute__((__always_inline__, __nodebug__, __target__("sse3"), __min_vector_width__(128)))
_mm_moveldup_ps(__m128 __a)
{
  return __builtin_shufflevector((__v4sf)__a, (__v4sf)__a, 0, 0, 2, 2);
}
# 168 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/pmmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse3"), __min_vector_width__(128)))
_mm_addsub_pd(__m128d __a, __m128d __b) {
  return __builtin_ia32_addsubpd((__v2df)__a, (__v2df)__b);
}
# 190 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/pmmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse3"), __min_vector_width__(128)))
_mm_hadd_pd(__m128d __a, __m128d __b) {
  return __builtin_ia32_haddpd((__v2df)__a, (__v2df)__b);
}
# 212 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/pmmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse3"), __min_vector_width__(128)))
_mm_hsub_pd(__m128d __a, __m128d __b) {
  return __builtin_ia32_hsubpd((__v2df)__a, (__v2df)__b);
}
# 247 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/pmmintrin.h"
static __inline__ __m128d __attribute__((__always_inline__, __nodebug__, __target__("sse3"), __min_vector_width__(128)))
_mm_movedup_pd(__m128d __a)
{
  return __builtin_shufflevector((__v2df)__a, (__v2df)__a, 0, 0);
}
# 271 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/pmmintrin.h"
static __inline__ void __attribute__((__always_inline__, __nodebug__, __target__("sse3"), __min_vector_width__(128)))
_mm_monitor(void const *__p, unsigned __extensions, unsigned __hints)
{
  __builtin_ia32_monitor(__p, __extensions, __hints);
}
# 293 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/pmmintrin.h"
static __inline__ void __attribute__((__always_inline__, __nodebug__, __target__("sse3"), __min_vector_width__(128)))
_mm_mwait(unsigned __extensions, unsigned __hints)
{
  __builtin_ia32_mwait(__extensions, __hints);
}
# 18 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/tmmintrin.h" 2
# 48 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/tmmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("ssse3"), __min_vector_width__(128))) _mm_abs_pi8(__m64 __a) {
  return (__m64)__builtin_elementwise_abs((__v8qs)__a);
}
# 64 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/tmmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("ssse3"), __min_vector_width__(128)))
_mm_abs_epi8(__m128i __a) {
  return (__m128i)__builtin_elementwise_abs((__v16qs)__a);
}
# 81 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/tmmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("ssse3"), __min_vector_width__(128))) _mm_abs_pi16(__m64 __a) {
  return (__m64)__builtin_elementwise_abs((__v4hi)__a);
}
# 97 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/tmmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("ssse3"), __min_vector_width__(128)))
_mm_abs_epi16(__m128i __a) {
  return (__m128i)__builtin_elementwise_abs((__v8hi)__a);
}
# 114 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/tmmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("ssse3"), __min_vector_width__(128))) _mm_abs_pi32(__m64 __a) {
  return (__m64)__builtin_elementwise_abs((__v2si)__a);
}
# 130 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/tmmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("ssse3"), __min_vector_width__(128)))
_mm_abs_epi32(__m128i __a) {
  return (__m128i)__builtin_elementwise_abs((__v4si)__a);
}
# 202 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/tmmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("ssse3"), __min_vector_width__(128)))
_mm_hadd_epi16(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_ia32_phaddw128((__v8hi)__a, (__v8hi)__b);
}
# 224 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/tmmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("ssse3"), __min_vector_width__(128)))
_mm_hadd_epi32(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_ia32_phaddd128((__v4si)__a, (__v4si)__b);
}
# 246 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/tmmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("ssse3"), __min_vector_width__(128))) _mm_hadd_pi16(__m64 __a,
                                                                   __m64 __b) {
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_phaddw128( (__v8hi)__builtin_shufflevector(__a, __b, 0, 1), (__v8hi){})), __extension__(__v2di){}, 0);

}
# 269 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/tmmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("ssse3"), __min_vector_width__(128))) _mm_hadd_pi32(__m64 __a,
                                                                   __m64 __b) {
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_phaddd128( (__v4si)__builtin_shufflevector(__a, __b, 0, 1), (__v4si){})), __extension__(__v2di){}, 0);

}
# 295 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/tmmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("ssse3"), __min_vector_width__(128)))
_mm_hadds_epi16(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_ia32_phaddsw128((__v8hi)__a, (__v8hi)__b);
}
# 320 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/tmmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("ssse3"), __min_vector_width__(128))) _mm_hadds_pi16(__m64 __a,
                                                                    __m64 __b) {
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_phaddsw128( (__v8hi)__builtin_shufflevector(__a, __b, 0, 1), (__v8hi){})), __extension__(__v2di){}, 0);

}
# 343 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/tmmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("ssse3"), __min_vector_width__(128)))
_mm_hsub_epi16(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_ia32_phsubw128((__v8hi)__a, (__v8hi)__b);
}
# 365 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/tmmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("ssse3"), __min_vector_width__(128)))
_mm_hsub_epi32(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_ia32_phsubd128((__v4si)__a, (__v4si)__b);
}
# 387 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/tmmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("ssse3"), __min_vector_width__(128))) _mm_hsub_pi16(__m64 __a,
                                                                   __m64 __b) {
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_phsubw128( (__v8hi)__builtin_shufflevector(__a, __b, 0, 1), (__v8hi){})), __extension__(__v2di){}, 0);

}
# 410 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/tmmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("ssse3"), __min_vector_width__(128))) _mm_hsub_pi32(__m64 __a,
                                                                   __m64 __b) {
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_phsubd128( (__v4si)__builtin_shufflevector(__a, __b, 0, 1), (__v4si){})), __extension__(__v2di){}, 0);

}
# 436 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/tmmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("ssse3"), __min_vector_width__(128)))
_mm_hsubs_epi16(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_ia32_phsubsw128((__v8hi)__a, (__v8hi)__b);
}
# 461 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/tmmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("ssse3"), __min_vector_width__(128))) _mm_hsubs_pi16(__m64 __a,
                                                                    __m64 __b) {
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_phsubsw128( (__v8hi)__builtin_shufflevector(__a, __b, 0, 1), (__v8hi){})), __extension__(__v2di){}, 0);

}
# 495 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/tmmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("ssse3"), __min_vector_width__(128)))
_mm_maddubs_epi16(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_ia32_pmaddubsw128((__v16qi)__a, (__v16qi)__b);
}
# 524 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/tmmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("ssse3"), __min_vector_width__(128)))
_mm_maddubs_pi16(__m64 __a, __m64 __b) {
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_pmaddubsw128((__v16qi)(__m128i) __builtin_shufflevector((__v2si)(__a), __extension__(__v2si){}, 0, 1, 2, 3), (__v16qi)(__m128i) __builtin_shufflevector((__v2si)(__b), __extension__(__v2si){}, 0, 1, 2, 3))), __extension__(__v2di){}, 0);

}
# 544 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/tmmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("ssse3"), __min_vector_width__(128)))
_mm_mulhrs_epi16(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_ia32_pmulhrsw128((__v8hi)__a, (__v8hi)__b);
}
# 563 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/tmmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("ssse3"), __min_vector_width__(128)))
_mm_mulhrs_pi16(__m64 __a, __m64 __b) {
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_pmulhrsw128((__v8hi)(__m128i) __builtin_shufflevector((__v2si)(__a), __extension__(__v2si){}, 0, 1, 2, 3), (__v8hi)(__m128i) __builtin_shufflevector((__v2si)(__b), __extension__(__v2si){}, 0, 1, 2, 3))), __extension__(__v2di){}, 0);

}
# 589 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/tmmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("ssse3"), __min_vector_width__(128)))
_mm_shuffle_epi8(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_ia32_pshufb128((__v16qi)__a, (__v16qi)__b);
}
# 613 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/tmmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("ssse3"), __min_vector_width__(128)))
_mm_shuffle_pi8(__m64 __a, __m64 __b) {
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_pshufb128( (__v16qi)__builtin_shufflevector((__v2si)(__a), __extension__(__v2si){}, 0, 1, 0, 1), (__v16qi)(__m128i) __builtin_shufflevector((__v2si)(__b), __extension__(__v2si){}, 0, 1, 2, 3))), __extension__(__v2di){}, 0);



}
# 641 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/tmmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("ssse3"), __min_vector_width__(128)))
_mm_sign_epi8(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_ia32_psignb128((__v16qi)__a, (__v16qi)__b);
}
# 666 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/tmmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("ssse3"), __min_vector_width__(128)))
_mm_sign_epi16(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_ia32_psignw128((__v8hi)__a, (__v8hi)__b);
}
# 691 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/tmmintrin.h"
static __inline__ __m128i __attribute__((__always_inline__, __nodebug__, __target__("ssse3"), __min_vector_width__(128)))
_mm_sign_epi32(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_ia32_psignd128((__v4si)__a, (__v4si)__b);
}
# 716 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/tmmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("ssse3"), __min_vector_width__(128))) _mm_sign_pi8(__m64 __a,
                                                                  __m64 __b) {
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_psignb128((__v16qi)(__m128i) __builtin_shufflevector((__v2si)(__a), __extension__(__v2si){}, 0, 1, 2, 3), (__v16qi)(__m128i) __builtin_shufflevector((__v2si)(__b), __extension__(__v2si){}, 0, 1, 2, 3))), __extension__(__v2di){}, 0);

}
# 742 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/tmmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("ssse3"), __min_vector_width__(128))) _mm_sign_pi16(__m64 __a,
                                                                   __m64 __b) {
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_psignw128((__v8hi)(__m128i) __builtin_shufflevector((__v2si)(__a), __extension__(__v2si){}, 0, 1, 2, 3), (__v8hi)(__m128i) __builtin_shufflevector((__v2si)(__b), __extension__(__v2si){}, 0, 1, 2, 3))), __extension__(__v2di){}, 0);

}
# 768 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/tmmintrin.h"
static __inline__ __m64 __attribute__((__always_inline__, __nodebug__, __target__("ssse3"), __min_vector_width__(128))) _mm_sign_pi32(__m64 __a,
                                                                   __m64 __b) {
  return (__m64) __builtin_shufflevector((__v2di)(__builtin_ia32_psignd128((__v4si)(__m128i) __builtin_shufflevector((__v2si)(__a), __extension__(__v2si){}, 0, 1, 2, 3), (__v4si)(__m128i) __builtin_shufflevector((__v2si)(__b), __extension__(__v2si){}, 0, 1, 2, 3))), __extension__(__v2di){}, 0);

}
# 4 "Sema/2010-05-31-palignr.c" 2
# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdint.h" 1
# 56 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdint.h"
# 1 "/usr/include/stdint.h" 1 3 4
# 26 "/usr/include/stdint.h" 3 4
# 1 "/usr/include/bits/wchar.h" 1 3 4
# 22 "/usr/include/bits/wchar.h" 3 4
# 1 "/usr/include/bits/wordsize.h" 1 3 4
# 23 "/usr/include/bits/wchar.h" 2 3 4
# 27 "/usr/include/stdint.h" 2 3 4
# 1 "/usr/include/bits/wordsize.h" 1 3 4
# 28 "/usr/include/stdint.h" 2 3 4
# 48 "/usr/include/stdint.h" 3 4
typedef unsigned char uint8_t;
typedef unsigned short int uint16_t;

typedef unsigned int uint32_t;



typedef unsigned long int uint64_t;
# 65 "/usr/include/stdint.h" 3 4
typedef signed char int_least8_t;
typedef short int int_least16_t;
typedef int int_least32_t;

typedef long int int_least64_t;






typedef unsigned char uint_least8_t;
typedef unsigned short int uint_least16_t;
typedef unsigned int uint_least32_t;

typedef unsigned long int uint_least64_t;
# 90 "/usr/include/stdint.h" 3 4
typedef signed char int_fast8_t;

typedef long int int_fast16_t;
typedef long int int_fast32_t;
typedef long int int_fast64_t;
# 103 "/usr/include/stdint.h" 3 4
typedef unsigned char uint_fast8_t;

typedef unsigned long int uint_fast16_t;
typedef unsigned long int uint_fast32_t;
typedef unsigned long int uint_fast64_t;
# 119 "/usr/include/stdint.h" 3 4
typedef long int intptr_t;


typedef unsigned long int uintptr_t;
# 134 "/usr/include/stdint.h" 3 4
typedef long int intmax_t;
typedef unsigned long int uintmax_t;
# 57 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdint.h" 2
# 5 "Sema/2010-05-31-palignr.c" 2

extern int i;

int main (void)
{
  typedef int16_t vSInt16 __attribute__ ((__vector_size__ (16)));

  short dtbl[] = {1,2,3,4,5,6,7,8};
  vSInt16 *vdtbl = (vSInt16*) dtbl;

  vSInt16 v0;
  v0 = *vdtbl;
  ((__m128i)__builtin_ia32_palignr128((__v16qi)(__m128i)(v0), (__v16qi)(__m128i)(v0), (i)));

  return 0;
}
