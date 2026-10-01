// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
# 1 "Sema/zvk-invalid-vlen.c"
# 1 "<built-in>" 1
# 1 "<built-in>" 3
# 412 "<built-in>" 3
# 1 "<command line>" 1
# 1 "<built-in>" 2
# 1 "Sema/zvk-invalid-vlen.c" 2




# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/riscv_vector.h" 1
# 14 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/riscv_vector.h"
# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdint.h" 1
# 56 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdint.h"
# 1 "/usr/include/stdint.h" 1 3 4
# 25 "/usr/include/stdint.h" 3 4
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
# 26 "/usr/include/stdint.h" 2 3 4
# 1 "/usr/include/bits/wchar.h" 1 3 4
# 22 "/usr/include/bits/wchar.h" 3 4
# 1 "/usr/include/bits/wordsize.h" 1 3 4
# 23 "/usr/include/bits/wchar.h" 2 3 4
# 27 "/usr/include/stdint.h" 2 3 4
# 1 "/usr/include/bits/wordsize.h" 1 3 4
# 28 "/usr/include/stdint.h" 2 3 4








typedef signed char int8_t;
typedef short int int16_t;
typedef int int32_t;

typedef long int int64_t;







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
# 15 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/riscv_vector.h" 2
# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h" 1
# 84 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h"
# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stddef_header_macro.h" 1
# 85 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h" 2



# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stddef_ptrdiff_t.h" 1
# 18 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stddef_ptrdiff_t.h"
typedef long int ptrdiff_t;
# 89 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h" 2




# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stddef_size_t.h" 1
# 18 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stddef_size_t.h"
typedef long unsigned int size_t;
# 94 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h" 2
# 103 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h"
# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stddef_wchar_t.h" 1
# 24 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stddef_wchar_t.h"
typedef int wchar_t;
# 104 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h" 2




# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stddef_null.h" 1
# 109 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h" 2
# 123 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h"
# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stddef_max_align_t.h" 1
# 19 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stddef_max_align_t.h"
typedef struct {
  long long __clang_max_align_nonce1
      __attribute__((__aligned__(__alignof__(long long))));
  long double __clang_max_align_nonce2
      __attribute__((__aligned__(__alignof__(long double))));
} max_align_t;
# 124 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h" 2




# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stddef_offsetof.h" 1
# 129 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h" 2
# 16 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/riscv_vector.h" 2





#pragma clang riscv intrinsic vector


enum __RISCV_FRM {
  __RISCV_FRM_RNE = 0,
  __RISCV_FRM_RTZ = 1,
  __RISCV_FRM_RDN = 2,
  __RISCV_FRM_RUP = 3,
  __RISCV_FRM_RMM = 4,
};
# 89 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/riscv_vector.h"
enum __RISCV_VXRM {
  __RISCV_VXRM_RNU = 0,
  __RISCV_VXRM_RNE = 1,
  __RISCV_VXRM_RDN = 2,
  __RISCV_VXRM_ROD = 3,
};
typedef __rvv_bool64_t vbool64_t;
typedef __rvv_bool32_t vbool32_t;
typedef __rvv_bool16_t vbool16_t;
typedef __rvv_bool8_t vbool8_t;
typedef __rvv_bool4_t vbool4_t;
typedef __rvv_bool2_t vbool2_t;
typedef __rvv_bool1_t vbool1_t;
typedef __rvv_int8mf8_t vint8mf8_t;
typedef __rvv_uint8mf8_t vuint8mf8_t;
typedef __rvv_int8mf8x2_t vint8mf8x2_t;
typedef __rvv_uint8mf8x2_t vuint8mf8x2_t;
typedef __rvv_int8mf8x3_t vint8mf8x3_t;
typedef __rvv_uint8mf8x3_t vuint8mf8x3_t;
typedef __rvv_int8mf8x4_t vint8mf8x4_t;
typedef __rvv_uint8mf8x4_t vuint8mf8x4_t;
typedef __rvv_int8mf8x5_t vint8mf8x5_t;
typedef __rvv_uint8mf8x5_t vuint8mf8x5_t;
typedef __rvv_int8mf8x6_t vint8mf8x6_t;
typedef __rvv_uint8mf8x6_t vuint8mf8x6_t;
typedef __rvv_int8mf8x7_t vint8mf8x7_t;
typedef __rvv_uint8mf8x7_t vuint8mf8x7_t;
typedef __rvv_int8mf8x8_t vint8mf8x8_t;
typedef __rvv_uint8mf8x8_t vuint8mf8x8_t;
typedef __rvv_int8mf4_t vint8mf4_t;
typedef __rvv_uint8mf4_t vuint8mf4_t;
typedef __rvv_int8mf4x2_t vint8mf4x2_t;
typedef __rvv_uint8mf4x2_t vuint8mf4x2_t;
typedef __rvv_int8mf4x3_t vint8mf4x3_t;
typedef __rvv_uint8mf4x3_t vuint8mf4x3_t;
typedef __rvv_int8mf4x4_t vint8mf4x4_t;
typedef __rvv_uint8mf4x4_t vuint8mf4x4_t;
typedef __rvv_int8mf4x5_t vint8mf4x5_t;
typedef __rvv_uint8mf4x5_t vuint8mf4x5_t;
typedef __rvv_int8mf4x6_t vint8mf4x6_t;
typedef __rvv_uint8mf4x6_t vuint8mf4x6_t;
typedef __rvv_int8mf4x7_t vint8mf4x7_t;
typedef __rvv_uint8mf4x7_t vuint8mf4x7_t;
typedef __rvv_int8mf4x8_t vint8mf4x8_t;
typedef __rvv_uint8mf4x8_t vuint8mf4x8_t;
typedef __rvv_int8mf2_t vint8mf2_t;
typedef __rvv_uint8mf2_t vuint8mf2_t;
typedef __rvv_int8mf2x2_t vint8mf2x2_t;
typedef __rvv_uint8mf2x2_t vuint8mf2x2_t;
typedef __rvv_int8mf2x3_t vint8mf2x3_t;
typedef __rvv_uint8mf2x3_t vuint8mf2x3_t;
typedef __rvv_int8mf2x4_t vint8mf2x4_t;
typedef __rvv_uint8mf2x4_t vuint8mf2x4_t;
typedef __rvv_int8mf2x5_t vint8mf2x5_t;
typedef __rvv_uint8mf2x5_t vuint8mf2x5_t;
typedef __rvv_int8mf2x6_t vint8mf2x6_t;
typedef __rvv_uint8mf2x6_t vuint8mf2x6_t;
typedef __rvv_int8mf2x7_t vint8mf2x7_t;
typedef __rvv_uint8mf2x7_t vuint8mf2x7_t;
typedef __rvv_int8mf2x8_t vint8mf2x8_t;
typedef __rvv_uint8mf2x8_t vuint8mf2x8_t;
typedef __rvv_int8m1_t vint8m1_t;
typedef __rvv_uint8m1_t vuint8m1_t;
typedef __rvv_int8m1x2_t vint8m1x2_t;
typedef __rvv_uint8m1x2_t vuint8m1x2_t;
typedef __rvv_int8m1x3_t vint8m1x3_t;
typedef __rvv_uint8m1x3_t vuint8m1x3_t;
typedef __rvv_int8m1x4_t vint8m1x4_t;
typedef __rvv_uint8m1x4_t vuint8m1x4_t;
typedef __rvv_int8m1x5_t vint8m1x5_t;
typedef __rvv_uint8m1x5_t vuint8m1x5_t;
typedef __rvv_int8m1x6_t vint8m1x6_t;
typedef __rvv_uint8m1x6_t vuint8m1x6_t;
typedef __rvv_int8m1x7_t vint8m1x7_t;
typedef __rvv_uint8m1x7_t vuint8m1x7_t;
typedef __rvv_int8m1x8_t vint8m1x8_t;
typedef __rvv_uint8m1x8_t vuint8m1x8_t;
typedef __rvv_int8m2_t vint8m2_t;
typedef __rvv_uint8m2_t vuint8m2_t;
typedef __rvv_int8m2x2_t vint8m2x2_t;
typedef __rvv_uint8m2x2_t vuint8m2x2_t;
typedef __rvv_int8m2x3_t vint8m2x3_t;
typedef __rvv_uint8m2x3_t vuint8m2x3_t;
typedef __rvv_int8m2x4_t vint8m2x4_t;
typedef __rvv_uint8m2x4_t vuint8m2x4_t;
typedef __rvv_int8m4_t vint8m4_t;
typedef __rvv_uint8m4_t vuint8m4_t;
typedef __rvv_int8m4x2_t vint8m4x2_t;
typedef __rvv_uint8m4x2_t vuint8m4x2_t;
typedef __rvv_int8m8_t vint8m8_t;
typedef __rvv_uint8m8_t vuint8m8_t;
typedef __rvv_int16mf4_t vint16mf4_t;
typedef __rvv_uint16mf4_t vuint16mf4_t;
typedef __rvv_int16mf4x2_t vint16mf4x2_t;
typedef __rvv_uint16mf4x2_t vuint16mf4x2_t;
typedef __rvv_int16mf4x3_t vint16mf4x3_t;
typedef __rvv_uint16mf4x3_t vuint16mf4x3_t;
typedef __rvv_int16mf4x4_t vint16mf4x4_t;
typedef __rvv_uint16mf4x4_t vuint16mf4x4_t;
typedef __rvv_int16mf4x5_t vint16mf4x5_t;
typedef __rvv_uint16mf4x5_t vuint16mf4x5_t;
typedef __rvv_int16mf4x6_t vint16mf4x6_t;
typedef __rvv_uint16mf4x6_t vuint16mf4x6_t;
typedef __rvv_int16mf4x7_t vint16mf4x7_t;
typedef __rvv_uint16mf4x7_t vuint16mf4x7_t;
typedef __rvv_int16mf4x8_t vint16mf4x8_t;
typedef __rvv_uint16mf4x8_t vuint16mf4x8_t;
typedef __rvv_int16mf2_t vint16mf2_t;
typedef __rvv_uint16mf2_t vuint16mf2_t;
typedef __rvv_int16mf2x2_t vint16mf2x2_t;
typedef __rvv_uint16mf2x2_t vuint16mf2x2_t;
typedef __rvv_int16mf2x3_t vint16mf2x3_t;
typedef __rvv_uint16mf2x3_t vuint16mf2x3_t;
typedef __rvv_int16mf2x4_t vint16mf2x4_t;
typedef __rvv_uint16mf2x4_t vuint16mf2x4_t;
typedef __rvv_int16mf2x5_t vint16mf2x5_t;
typedef __rvv_uint16mf2x5_t vuint16mf2x5_t;
typedef __rvv_int16mf2x6_t vint16mf2x6_t;
typedef __rvv_uint16mf2x6_t vuint16mf2x6_t;
typedef __rvv_int16mf2x7_t vint16mf2x7_t;
typedef __rvv_uint16mf2x7_t vuint16mf2x7_t;
typedef __rvv_int16mf2x8_t vint16mf2x8_t;
typedef __rvv_uint16mf2x8_t vuint16mf2x8_t;
typedef __rvv_int16m1_t vint16m1_t;
typedef __rvv_uint16m1_t vuint16m1_t;
typedef __rvv_int16m1x2_t vint16m1x2_t;
typedef __rvv_uint16m1x2_t vuint16m1x2_t;
typedef __rvv_int16m1x3_t vint16m1x3_t;
typedef __rvv_uint16m1x3_t vuint16m1x3_t;
typedef __rvv_int16m1x4_t vint16m1x4_t;
typedef __rvv_uint16m1x4_t vuint16m1x4_t;
typedef __rvv_int16m1x5_t vint16m1x5_t;
typedef __rvv_uint16m1x5_t vuint16m1x5_t;
typedef __rvv_int16m1x6_t vint16m1x6_t;
typedef __rvv_uint16m1x6_t vuint16m1x6_t;
typedef __rvv_int16m1x7_t vint16m1x7_t;
typedef __rvv_uint16m1x7_t vuint16m1x7_t;
typedef __rvv_int16m1x8_t vint16m1x8_t;
typedef __rvv_uint16m1x8_t vuint16m1x8_t;
typedef __rvv_int16m2_t vint16m2_t;
typedef __rvv_uint16m2_t vuint16m2_t;
typedef __rvv_int16m2x2_t vint16m2x2_t;
typedef __rvv_uint16m2x2_t vuint16m2x2_t;
typedef __rvv_int16m2x3_t vint16m2x3_t;
typedef __rvv_uint16m2x3_t vuint16m2x3_t;
typedef __rvv_int16m2x4_t vint16m2x4_t;
typedef __rvv_uint16m2x4_t vuint16m2x4_t;
typedef __rvv_int16m4_t vint16m4_t;
typedef __rvv_uint16m4_t vuint16m4_t;
typedef __rvv_int16m4x2_t vint16m4x2_t;
typedef __rvv_uint16m4x2_t vuint16m4x2_t;
typedef __rvv_int16m8_t vint16m8_t;
typedef __rvv_uint16m8_t vuint16m8_t;
typedef __rvv_int32mf2_t vint32mf2_t;
typedef __rvv_uint32mf2_t vuint32mf2_t;
typedef __rvv_int32mf2x2_t vint32mf2x2_t;
typedef __rvv_uint32mf2x2_t vuint32mf2x2_t;
typedef __rvv_int32mf2x3_t vint32mf2x3_t;
typedef __rvv_uint32mf2x3_t vuint32mf2x3_t;
typedef __rvv_int32mf2x4_t vint32mf2x4_t;
typedef __rvv_uint32mf2x4_t vuint32mf2x4_t;
typedef __rvv_int32mf2x5_t vint32mf2x5_t;
typedef __rvv_uint32mf2x5_t vuint32mf2x5_t;
typedef __rvv_int32mf2x6_t vint32mf2x6_t;
typedef __rvv_uint32mf2x6_t vuint32mf2x6_t;
typedef __rvv_int32mf2x7_t vint32mf2x7_t;
typedef __rvv_uint32mf2x7_t vuint32mf2x7_t;
typedef __rvv_int32mf2x8_t vint32mf2x8_t;
typedef __rvv_uint32mf2x8_t vuint32mf2x8_t;
typedef __rvv_int32m1_t vint32m1_t;
typedef __rvv_uint32m1_t vuint32m1_t;
typedef __rvv_int32m1x2_t vint32m1x2_t;
typedef __rvv_uint32m1x2_t vuint32m1x2_t;
typedef __rvv_int32m1x3_t vint32m1x3_t;
typedef __rvv_uint32m1x3_t vuint32m1x3_t;
typedef __rvv_int32m1x4_t vint32m1x4_t;
typedef __rvv_uint32m1x4_t vuint32m1x4_t;
typedef __rvv_int32m1x5_t vint32m1x5_t;
typedef __rvv_uint32m1x5_t vuint32m1x5_t;
typedef __rvv_int32m1x6_t vint32m1x6_t;
typedef __rvv_uint32m1x6_t vuint32m1x6_t;
typedef __rvv_int32m1x7_t vint32m1x7_t;
typedef __rvv_uint32m1x7_t vuint32m1x7_t;
typedef __rvv_int32m1x8_t vint32m1x8_t;
typedef __rvv_uint32m1x8_t vuint32m1x8_t;
typedef __rvv_int32m2_t vint32m2_t;
typedef __rvv_uint32m2_t vuint32m2_t;
typedef __rvv_int32m2x2_t vint32m2x2_t;
typedef __rvv_uint32m2x2_t vuint32m2x2_t;
typedef __rvv_int32m2x3_t vint32m2x3_t;
typedef __rvv_uint32m2x3_t vuint32m2x3_t;
typedef __rvv_int32m2x4_t vint32m2x4_t;
typedef __rvv_uint32m2x4_t vuint32m2x4_t;
typedef __rvv_int32m4_t vint32m4_t;
typedef __rvv_uint32m4_t vuint32m4_t;
typedef __rvv_int32m4x2_t vint32m4x2_t;
typedef __rvv_uint32m4x2_t vuint32m4x2_t;
typedef __rvv_int32m8_t vint32m8_t;
typedef __rvv_uint32m8_t vuint32m8_t;
typedef __rvv_int64m1_t vint64m1_t;
typedef __rvv_uint64m1_t vuint64m1_t;
typedef __rvv_int64m1x2_t vint64m1x2_t;
typedef __rvv_uint64m1x2_t vuint64m1x2_t;
typedef __rvv_int64m1x3_t vint64m1x3_t;
typedef __rvv_uint64m1x3_t vuint64m1x3_t;
typedef __rvv_int64m1x4_t vint64m1x4_t;
typedef __rvv_uint64m1x4_t vuint64m1x4_t;
typedef __rvv_int64m1x5_t vint64m1x5_t;
typedef __rvv_uint64m1x5_t vuint64m1x5_t;
typedef __rvv_int64m1x6_t vint64m1x6_t;
typedef __rvv_uint64m1x6_t vuint64m1x6_t;
typedef __rvv_int64m1x7_t vint64m1x7_t;
typedef __rvv_uint64m1x7_t vuint64m1x7_t;
typedef __rvv_int64m1x8_t vint64m1x8_t;
typedef __rvv_uint64m1x8_t vuint64m1x8_t;
typedef __rvv_int64m2_t vint64m2_t;
typedef __rvv_uint64m2_t vuint64m2_t;
typedef __rvv_int64m2x2_t vint64m2x2_t;
typedef __rvv_uint64m2x2_t vuint64m2x2_t;
typedef __rvv_int64m2x3_t vint64m2x3_t;
typedef __rvv_uint64m2x3_t vuint64m2x3_t;
typedef __rvv_int64m2x4_t vint64m2x4_t;
typedef __rvv_uint64m2x4_t vuint64m2x4_t;
typedef __rvv_int64m4_t vint64m4_t;
typedef __rvv_uint64m4_t vuint64m4_t;
typedef __rvv_int64m4x2_t vint64m4x2_t;
typedef __rvv_uint64m4x2_t vuint64m4x2_t;
typedef __rvv_int64m8_t vint64m8_t;
typedef __rvv_uint64m8_t vuint64m8_t;
typedef __rvv_float16mf4_t vfloat16mf4_t;
typedef __rvv_float16mf4x2_t vfloat16mf4x2_t;
typedef __rvv_float16mf4x3_t vfloat16mf4x3_t;
typedef __rvv_float16mf4x4_t vfloat16mf4x4_t;
typedef __rvv_float16mf4x5_t vfloat16mf4x5_t;
typedef __rvv_float16mf4x6_t vfloat16mf4x6_t;
typedef __rvv_float16mf4x7_t vfloat16mf4x7_t;
typedef __rvv_float16mf4x8_t vfloat16mf4x8_t;
typedef __rvv_float16mf2_t vfloat16mf2_t;
typedef __rvv_float16mf2x2_t vfloat16mf2x2_t;
typedef __rvv_float16mf2x3_t vfloat16mf2x3_t;
typedef __rvv_float16mf2x4_t vfloat16mf2x4_t;
typedef __rvv_float16mf2x5_t vfloat16mf2x5_t;
typedef __rvv_float16mf2x6_t vfloat16mf2x6_t;
typedef __rvv_float16mf2x7_t vfloat16mf2x7_t;
typedef __rvv_float16mf2x8_t vfloat16mf2x8_t;
typedef __rvv_float16m1_t vfloat16m1_t;
typedef __rvv_float16m1x2_t vfloat16m1x2_t;
typedef __rvv_float16m1x3_t vfloat16m1x3_t;
typedef __rvv_float16m1x4_t vfloat16m1x4_t;
typedef __rvv_float16m1x5_t vfloat16m1x5_t;
typedef __rvv_float16m1x6_t vfloat16m1x6_t;
typedef __rvv_float16m1x7_t vfloat16m1x7_t;
typedef __rvv_float16m1x8_t vfloat16m1x8_t;
typedef __rvv_float16m2_t vfloat16m2_t;
typedef __rvv_float16m2x2_t vfloat16m2x2_t;
typedef __rvv_float16m2x3_t vfloat16m2x3_t;
typedef __rvv_float16m2x4_t vfloat16m2x4_t;
typedef __rvv_float16m4_t vfloat16m4_t;
typedef __rvv_float16m4x2_t vfloat16m4x2_t;
typedef __rvv_float16m8_t vfloat16m8_t;
typedef __rvv_float32mf2_t vfloat32mf2_t;
typedef __rvv_float32mf2x2_t vfloat32mf2x2_t;
typedef __rvv_float32mf2x3_t vfloat32mf2x3_t;
typedef __rvv_float32mf2x4_t vfloat32mf2x4_t;
typedef __rvv_float32mf2x5_t vfloat32mf2x5_t;
typedef __rvv_float32mf2x6_t vfloat32mf2x6_t;
typedef __rvv_float32mf2x7_t vfloat32mf2x7_t;
typedef __rvv_float32mf2x8_t vfloat32mf2x8_t;
typedef __rvv_float32m1_t vfloat32m1_t;
typedef __rvv_float32m1x2_t vfloat32m1x2_t;
typedef __rvv_float32m1x3_t vfloat32m1x3_t;
typedef __rvv_float32m1x4_t vfloat32m1x4_t;
typedef __rvv_float32m1x5_t vfloat32m1x5_t;
typedef __rvv_float32m1x6_t vfloat32m1x6_t;
typedef __rvv_float32m1x7_t vfloat32m1x7_t;
typedef __rvv_float32m1x8_t vfloat32m1x8_t;
typedef __rvv_float32m2_t vfloat32m2_t;
typedef __rvv_float32m2x2_t vfloat32m2x2_t;
typedef __rvv_float32m2x3_t vfloat32m2x3_t;
typedef __rvv_float32m2x4_t vfloat32m2x4_t;
typedef __rvv_float32m4_t vfloat32m4_t;
typedef __rvv_float32m4x2_t vfloat32m4x2_t;
typedef __rvv_float32m8_t vfloat32m8_t;
typedef __rvv_float64m1_t vfloat64m1_t;
typedef __rvv_float64m1x2_t vfloat64m1x2_t;
typedef __rvv_float64m1x3_t vfloat64m1x3_t;
typedef __rvv_float64m1x4_t vfloat64m1x4_t;
typedef __rvv_float64m1x5_t vfloat64m1x5_t;
typedef __rvv_float64m1x6_t vfloat64m1x6_t;
typedef __rvv_float64m1x7_t vfloat64m1x7_t;
typedef __rvv_float64m1x8_t vfloat64m1x8_t;
typedef __rvv_float64m2_t vfloat64m2_t;
typedef __rvv_float64m2x2_t vfloat64m2x2_t;
typedef __rvv_float64m2x3_t vfloat64m2x3_t;
typedef __rvv_float64m2x4_t vfloat64m2x4_t;
typedef __rvv_float64m4_t vfloat64m4_t;
typedef __rvv_float64m4x2_t vfloat64m4x2_t;
typedef __rvv_float64m8_t vfloat64m8_t;
typedef __rvv_bfloat16mf4_t vbfloat16mf4_t;
typedef __rvv_bfloat16mf4x2_t vbfloat16mf4x2_t;
typedef __rvv_bfloat16mf4x3_t vbfloat16mf4x3_t;
typedef __rvv_bfloat16mf4x4_t vbfloat16mf4x4_t;
typedef __rvv_bfloat16mf4x5_t vbfloat16mf4x5_t;
typedef __rvv_bfloat16mf4x6_t vbfloat16mf4x6_t;
typedef __rvv_bfloat16mf4x7_t vbfloat16mf4x7_t;
typedef __rvv_bfloat16mf4x8_t vbfloat16mf4x8_t;
typedef __rvv_bfloat16mf2_t vbfloat16mf2_t;
typedef __rvv_bfloat16mf2x2_t vbfloat16mf2x2_t;
typedef __rvv_bfloat16mf2x3_t vbfloat16mf2x3_t;
typedef __rvv_bfloat16mf2x4_t vbfloat16mf2x4_t;
typedef __rvv_bfloat16mf2x5_t vbfloat16mf2x5_t;
typedef __rvv_bfloat16mf2x6_t vbfloat16mf2x6_t;
typedef __rvv_bfloat16mf2x7_t vbfloat16mf2x7_t;
typedef __rvv_bfloat16mf2x8_t vbfloat16mf2x8_t;
typedef __rvv_bfloat16m1_t vbfloat16m1_t;
typedef __rvv_bfloat16m1x2_t vbfloat16m1x2_t;
typedef __rvv_bfloat16m1x3_t vbfloat16m1x3_t;
typedef __rvv_bfloat16m1x4_t vbfloat16m1x4_t;
typedef __rvv_bfloat16m1x5_t vbfloat16m1x5_t;
typedef __rvv_bfloat16m1x6_t vbfloat16m1x6_t;
typedef __rvv_bfloat16m1x7_t vbfloat16m1x7_t;
typedef __rvv_bfloat16m1x8_t vbfloat16m1x8_t;
typedef __rvv_bfloat16m2_t vbfloat16m2_t;
typedef __rvv_bfloat16m2x2_t vbfloat16m2x2_t;
typedef __rvv_bfloat16m2x3_t vbfloat16m2x3_t;
typedef __rvv_bfloat16m2x4_t vbfloat16m2x4_t;
typedef __rvv_bfloat16m4_t vbfloat16m4_t;
typedef __rvv_bfloat16m4x2_t vbfloat16m4x2_t;
typedef __rvv_bfloat16m8_t vbfloat16m8_t;
# 6 "Sema/zvk-invalid-vlen.c" 2

void test_vaeskf1_tu(vuint32mf2_t vd, vuint32mf2_t vs2, size_t vl) {
  __riscv_vaeskf1_vi_u32mf2_tu(vd, vs2, 0, vl);
}

void test_vsm3c_tu(vuint32mf2_t vd, vuint32mf2_t vs2, size_t vl) {
  __riscv_vsm3c_vi_u32mf2_tu(vd, vs2, 0, vl);
}

void test_vaeskf1(vuint32mf2_t vs2, size_t vl) {
  __riscv_vaeskf1_vi_u32mf2(vs2, 0, vl);
}

void test_vaesdf(vuint32mf2_t vd, vuint32mf2_t vs2, size_t vl) {
  __riscv_vaesdf_vv_u32mf2(vd, vs2, vl);
}

void test_vaesdf_vs(vuint32m2_t vd, vuint32mf2_t vs2, size_t vl) {
  __riscv_vaesdf_vs_u32mf2_u32m2(vd, vs2, vl);
}
