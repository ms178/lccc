// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c --target linux_aarch64
# 1 "Sema/attr-arm-sve-vector-bits.c"
# 1 "<built-in>" 1
# 1 "<built-in>" 3
# 412 "<built-in>" 3
# 1 "<command line>" 1
# 1 "<built-in>" 2
# 1 "Sema/attr-arm-sve-vector-bits.c" 2






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
# 8 "Sema/attr-arm-sve-vector-bits.c" 2



typedef __fp16 float16_t;
typedef float float32_t;
typedef double float64_t;
typedef __SVInt8_t svint8_t;
typedef __SVInt16_t svint16_t;
typedef __SVInt32_t svint32_t;
typedef __SVInt64_t svint64_t;
typedef __SVUint8_t svuint8_t;
typedef __SVUint16_t svuint16_t;
typedef __SVUint32_t svuint32_t;
typedef __SVUint64_t svuint64_t;
typedef __SVFloat16_t svfloat16_t;
typedef __SVFloat32_t svfloat32_t;
typedef __SVFloat64_t svfloat64_t;






typedef __SVBool_t svbool_t;


typedef svint8_t fixed_int8_t __attribute__((arm_sve_vector_bits(__ARM_FEATURE_SVE_BITS)));
typedef svint16_t fixed_int16_t __attribute__((arm_sve_vector_bits(__ARM_FEATURE_SVE_BITS)));
typedef svint32_t fixed_int32_t __attribute__((arm_sve_vector_bits(__ARM_FEATURE_SVE_BITS)));
typedef svint64_t fixed_int64_t __attribute__((arm_sve_vector_bits(__ARM_FEATURE_SVE_BITS)));

typedef svuint8_t fixed_uint8_t __attribute__((arm_sve_vector_bits(__ARM_FEATURE_SVE_BITS)));
typedef svuint16_t fixed_uint16_t __attribute__((arm_sve_vector_bits(__ARM_FEATURE_SVE_BITS)));
typedef svuint32_t fixed_uint32_t __attribute__((arm_sve_vector_bits(__ARM_FEATURE_SVE_BITS)));
typedef svuint64_t fixed_uint64_t __attribute__((arm_sve_vector_bits(__ARM_FEATURE_SVE_BITS)));

typedef svfloat16_t fixed_float16_t __attribute__((arm_sve_vector_bits(__ARM_FEATURE_SVE_BITS)));
typedef svfloat32_t fixed_float32_t __attribute__((arm_sve_vector_bits(__ARM_FEATURE_SVE_BITS)));
typedef svfloat64_t fixed_float64_t __attribute__((arm_sve_vector_bits(__ARM_FEATURE_SVE_BITS)));

typedef svbfloat16_t fixed_bfloat16_t __attribute__((arm_sve_vector_bits(__ARM_FEATURE_SVE_BITS)));

typedef svbool_t fixed_bool_t __attribute__((arm_sve_vector_bits(__ARM_FEATURE_SVE_BITS)));


typedef int8_t gnu_int8_t __attribute__((vector_size(__ARM_FEATURE_SVE_BITS / 8)));
typedef int16_t gnu_int16_t __attribute__((vector_size(__ARM_FEATURE_SVE_BITS / 8)));
typedef int32_t gnu_int32_t __attribute__((vector_size(__ARM_FEATURE_SVE_BITS / 8)));
typedef int64_t gnu_int64_t __attribute__((vector_size(__ARM_FEATURE_SVE_BITS / 8)));

typedef uint8_t gnu_uint8_t __attribute__((vector_size(__ARM_FEATURE_SVE_BITS / 8)));
typedef uint16_t gnu_uint16_t __attribute__((vector_size(__ARM_FEATURE_SVE_BITS / 8)));
typedef uint32_t gnu_uint32_t __attribute__((vector_size(__ARM_FEATURE_SVE_BITS / 8)));
typedef uint64_t gnu_uint64_t __attribute__((vector_size(__ARM_FEATURE_SVE_BITS / 8)));

typedef float16_t gnu_float16_t __attribute__((vector_size(__ARM_FEATURE_SVE_BITS / 8)));
typedef float32_t gnu_float32_t __attribute__((vector_size(__ARM_FEATURE_SVE_BITS / 8)));
typedef float64_t gnu_float64_t __attribute__((vector_size(__ARM_FEATURE_SVE_BITS / 8)));

typedef bfloat16_t gnu_bfloat16_t __attribute__((vector_size(__ARM_FEATURE_SVE_BITS / 8)));


typedef svint8_t no_argument __attribute__((arm_sve_vector_bits));
typedef svint8_t two_arguments __attribute__((arm_sve_vector_bits(2, 4)));


typedef svint8_t non_int_size1 __attribute__((arm_sve_vector_bits(2.0)));
typedef svint8_t non_int_size2 __attribute__((arm_sve_vector_bits("256")));

typedef __clang_svint8x2_t svint8x2_t;
typedef __clang_svfloat32x3_t svfloat32x3_t;


typedef void *badtype1 __attribute__((arm_sve_vector_bits(__ARM_FEATURE_SVE_BITS)));
typedef int badtype2 __attribute__((arm_sve_vector_bits(__ARM_FEATURE_SVE_BITS)));
typedef float badtype3 __attribute__((arm_sve_vector_bits(__ARM_FEATURE_SVE_BITS)));
typedef svint8x2_t badtype4 __attribute__((arm_sve_vector_bits(__ARM_FEATURE_SVE_BITS)));
typedef svfloat32x3_t badtype5 __attribute__((arm_sve_vector_bits(__ARM_FEATURE_SVE_BITS)));


svint8_t non_typedef_type __attribute__((arm_sve_vector_bits(__ARM_FEATURE_SVE_BITS)));



fixed_int8_t global_int8;
fixed_bfloat16_t global_bfloat16;
fixed_bool_t global_bool;

extern fixed_int8_t extern_int8;
extern fixed_bfloat16_t extern_bfloat16;
extern fixed_bool_t extern_bool;

static fixed_int8_t static_int8;
static fixed_bfloat16_t static_bfloat16;
static fixed_bool_t static_bool;

fixed_int8_t *global_int8_ptr;
extern fixed_int8_t *extern_int8_ptr;
static fixed_int8_t *static_int8_ptr;
__thread fixed_int8_t thread_int8;

typedef fixed_int8_t int8_typedef;
typedef fixed_int8_t *int8_ptr_typedef;


int sizeof_int8 = sizeof(global_int8);
int sizeof_int8_var = sizeof(*global_int8_ptr);
int sizeof_int8_var_ptr = sizeof(global_int8_ptr);

extern fixed_int8_t *extern_int8_ptr;

int alignof_int8 = __alignof__(extern_int8);
int alignof_int8_var = __alignof__(*extern_int8_ptr);
int alignof_int8_var_ptr = __alignof__(extern_int8_ptr);

void f(int c) {
  fixed_int8_t fs8;
  svint8_t ss8;
  gnu_int8_t gs8;



  void *sel __attribute__((unused));
  sel = c ? ss8 : fs8;
  sel = c ? fs8 : ss8;

  sel = c ? gs8 : ss8;
  sel = c ? ss8 : gs8;

  sel = c ? gs8 : fs8;
  sel = c ? fs8 : gs8;


  ss8 = ss8 + fs8;
  ss8 = ss8 + gs8;

  fs8 = fs8 + ss8;
  fs8 = fs8 + gs8;

  gs8 = gs8 + ss8;
  gs8 = gs8 + fs8;

  ss8 += fs8;
  ss8 += gs8;

  fs8 += ss8;
  fs8 += gs8;

  gs8 += ss8;
  gs8 += fs8;

  ss8 = ss8 == fs8;
  ss8 = ss8 == gs8;

  fs8 = fs8 == ss8;
  fs8 = fs8 == gs8;

  gs8 = gs8 == ss8;
  gs8 = gs8 == fs8;

  ss8 = ss8 & fs8;
  ss8 = ss8 & gs8;

  fs8 = fs8 & ss8;
  fs8 = fs8 & gs8;

  gs8 = gs8 & ss8;
  gs8 = gs8 & fs8;
}







_Static_assert(sizeof(fixed_int8_t) == ((__ARM_FEATURE_SVE_BITS / 8)), "");

_Static_assert(sizeof(fixed_int16_t) == ((__ARM_FEATURE_SVE_BITS / 8)), "");
_Static_assert(sizeof(fixed_int32_t) == ((__ARM_FEATURE_SVE_BITS / 8)), "");
_Static_assert(sizeof(fixed_int64_t) == ((__ARM_FEATURE_SVE_BITS / 8)), "");

_Static_assert(sizeof(fixed_uint8_t) == ((__ARM_FEATURE_SVE_BITS / 8)), "");
_Static_assert(sizeof(fixed_uint16_t) == ((__ARM_FEATURE_SVE_BITS / 8)), "");
_Static_assert(sizeof(fixed_uint32_t) == ((__ARM_FEATURE_SVE_BITS / 8)), "");
_Static_assert(sizeof(fixed_uint64_t) == ((__ARM_FEATURE_SVE_BITS / 8)), "");

_Static_assert(sizeof(fixed_float16_t) == ((__ARM_FEATURE_SVE_BITS / 8)), "");
_Static_assert(sizeof(fixed_float32_t) == ((__ARM_FEATURE_SVE_BITS / 8)), "");
_Static_assert(sizeof(fixed_float64_t) == ((__ARM_FEATURE_SVE_BITS / 8)), "");

_Static_assert(sizeof(fixed_bfloat16_t) == ((__ARM_FEATURE_SVE_BITS / 8)), "");

_Static_assert(sizeof(fixed_bool_t) == ((__ARM_FEATURE_SVE_BITS / 64)), "");







_Static_assert(__alignof__(fixed_int8_t) == 16, "");
_Static_assert(__alignof__(fixed_int16_t) == 16, "");
_Static_assert(__alignof__(fixed_int32_t) == 16, "");
_Static_assert(__alignof__(fixed_int64_t) == 16, "");

_Static_assert(__alignof__(fixed_uint8_t) == 16, "");
_Static_assert(__alignof__(fixed_uint16_t) == 16, "");
_Static_assert(__alignof__(fixed_uint32_t) == 16, "");
_Static_assert(__alignof__(fixed_uint64_t) == 16, "");

_Static_assert(__alignof__(fixed_float16_t) == 16, "");
_Static_assert(__alignof__(fixed_float32_t) == 16, "");
_Static_assert(__alignof__(fixed_float64_t) == 16, "");

_Static_assert(__alignof__(fixed_bfloat16_t) == 16, "");

_Static_assert(__alignof__(fixed_bool_t) == 2, "");




struct struct_int64 { fixed_int64_t x, y[5]; };
struct struct_float64 { fixed_float64_t x, y[5]; };
struct struct_bfloat16 { fixed_bfloat16_t x, y[5]; };
struct struct_bool { fixed_bool_t x, y[5]; };



union union_int64 { fixed_int64_t x, y[5]; };
union union_float64 { fixed_float64_t x, y[5]; };
union union_bfloat16 { fixed_bfloat16_t x, y[5]; };
union union_bool { fixed_bool_t x, y[5]; };
# 258 "Sema/attr-arm-sve-vector-bits.c"
svint8_t to_svint8_t_from_fixed(fixed_int8_t x) { return x; } fixed_int8_t from_svint8_t_to_fixed(svint8_t x) { return x; } gnu_int8_t to_gnu_int8_t_from_svint8_t(svint8_t x) { return x; } svint8_t from_gnu_int8_t_to_svint8_t(gnu_int8_t x) { return x; } gnu_int8_t to_gnu_int8_t_from_fixed_int8_t(fixed_int8_t x) { return x; } fixed_int8_t from_gnu_int8_t_to_fixed_int8_t(gnu_int8_t x) { return x; }
svint16_t to_svint16_t_from_fixed(fixed_int16_t x) { return x; } fixed_int16_t from_svint16_t_to_fixed(svint16_t x) { return x; } gnu_int16_t to_gnu_int16_t_from_svint16_t(svint16_t x) { return x; } svint16_t from_gnu_int16_t_to_svint16_t(gnu_int16_t x) { return x; } gnu_int16_t to_gnu_int16_t_from_fixed_int16_t(fixed_int16_t x) { return x; } fixed_int16_t from_gnu_int16_t_to_fixed_int16_t(gnu_int16_t x) { return x; }
svint32_t to_svint32_t_from_fixed(fixed_int32_t x) { return x; } fixed_int32_t from_svint32_t_to_fixed(svint32_t x) { return x; } gnu_int32_t to_gnu_int32_t_from_svint32_t(svint32_t x) { return x; } svint32_t from_gnu_int32_t_to_svint32_t(gnu_int32_t x) { return x; } gnu_int32_t to_gnu_int32_t_from_fixed_int32_t(fixed_int32_t x) { return x; } fixed_int32_t from_gnu_int32_t_to_fixed_int32_t(gnu_int32_t x) { return x; }
svint64_t to_svint64_t_from_fixed(fixed_int64_t x) { return x; } fixed_int64_t from_svint64_t_to_fixed(svint64_t x) { return x; } gnu_int64_t to_gnu_int64_t_from_svint64_t(svint64_t x) { return x; } svint64_t from_gnu_int64_t_to_svint64_t(gnu_int64_t x) { return x; } gnu_int64_t to_gnu_int64_t_from_fixed_int64_t(fixed_int64_t x) { return x; } fixed_int64_t from_gnu_int64_t_to_fixed_int64_t(gnu_int64_t x) { return x; }
svuint8_t to_svuint8_t_from_fixed(fixed_uint8_t x) { return x; } fixed_uint8_t from_svuint8_t_to_fixed(svuint8_t x) { return x; } gnu_uint8_t to_gnu_uint8_t_from_svuint8_t(svuint8_t x) { return x; } svuint8_t from_gnu_uint8_t_to_svuint8_t(gnu_uint8_t x) { return x; } gnu_uint8_t to_gnu_uint8_t_from_fixed_uint8_t(fixed_uint8_t x) { return x; } fixed_uint8_t from_gnu_uint8_t_to_fixed_uint8_t(gnu_uint8_t x) { return x; }
svuint16_t to_svuint16_t_from_fixed(fixed_uint16_t x) { return x; } fixed_uint16_t from_svuint16_t_to_fixed(svuint16_t x) { return x; } gnu_uint16_t to_gnu_uint16_t_from_svuint16_t(svuint16_t x) { return x; } svuint16_t from_gnu_uint16_t_to_svuint16_t(gnu_uint16_t x) { return x; } gnu_uint16_t to_gnu_uint16_t_from_fixed_uint16_t(fixed_uint16_t x) { return x; } fixed_uint16_t from_gnu_uint16_t_to_fixed_uint16_t(gnu_uint16_t x) { return x; }
svuint32_t to_svuint32_t_from_fixed(fixed_uint32_t x) { return x; } fixed_uint32_t from_svuint32_t_to_fixed(svuint32_t x) { return x; } gnu_uint32_t to_gnu_uint32_t_from_svuint32_t(svuint32_t x) { return x; } svuint32_t from_gnu_uint32_t_to_svuint32_t(gnu_uint32_t x) { return x; } gnu_uint32_t to_gnu_uint32_t_from_fixed_uint32_t(fixed_uint32_t x) { return x; } fixed_uint32_t from_gnu_uint32_t_to_fixed_uint32_t(gnu_uint32_t x) { return x; }
svuint64_t to_svuint64_t_from_fixed(fixed_uint64_t x) { return x; } fixed_uint64_t from_svuint64_t_to_fixed(svuint64_t x) { return x; } gnu_uint64_t to_gnu_uint64_t_from_svuint64_t(svuint64_t x) { return x; } svuint64_t from_gnu_uint64_t_to_svuint64_t(gnu_uint64_t x) { return x; } gnu_uint64_t to_gnu_uint64_t_from_fixed_uint64_t(fixed_uint64_t x) { return x; } fixed_uint64_t from_gnu_uint64_t_to_fixed_uint64_t(gnu_uint64_t x) { return x; }
svfloat16_t to_svfloat16_t_from_fixed(fixed_float16_t x) { return x; } fixed_float16_t from_svfloat16_t_to_fixed(svfloat16_t x) { return x; } gnu_float16_t to_gnu_float16_t_from_svfloat16_t(svfloat16_t x) { return x; } svfloat16_t from_gnu_float16_t_to_svfloat16_t(gnu_float16_t x) { return x; } gnu_float16_t to_gnu_float16_t_from_fixed_float16_t(fixed_float16_t x) { return x; } fixed_float16_t from_gnu_float16_t_to_fixed_float16_t(gnu_float16_t x) { return x; }
svfloat32_t to_svfloat32_t_from_fixed(fixed_float32_t x) { return x; } fixed_float32_t from_svfloat32_t_to_fixed(svfloat32_t x) { return x; } gnu_float32_t to_gnu_float32_t_from_svfloat32_t(svfloat32_t x) { return x; } svfloat32_t from_gnu_float32_t_to_svfloat32_t(gnu_float32_t x) { return x; } gnu_float32_t to_gnu_float32_t_from_fixed_float32_t(fixed_float32_t x) { return x; } fixed_float32_t from_gnu_float32_t_to_fixed_float32_t(gnu_float32_t x) { return x; }
svfloat64_t to_svfloat64_t_from_fixed(fixed_float64_t x) { return x; } fixed_float64_t from_svfloat64_t_to_fixed(svfloat64_t x) { return x; } gnu_float64_t to_gnu_float64_t_from_svfloat64_t(svfloat64_t x) { return x; } svfloat64_t from_gnu_float64_t_to_svfloat64_t(gnu_float64_t x) { return x; } gnu_float64_t to_gnu_float64_t_from_fixed_float64_t(fixed_float64_t x) { return x; } fixed_float64_t from_gnu_float64_t_to_fixed_float64_t(gnu_float64_t x) { return x; }
svbfloat16_t to_svbfloat16_t_from_fixed(fixed_bfloat16_t x) { return x; } fixed_bfloat16_t from_svbfloat16_t_to_fixed(svbfloat16_t x) { return x; } gnu_bfloat16_t to_gnu_bfloat16_t_from_svbfloat16_t(svbfloat16_t x) { return x; } svbfloat16_t from_gnu_bfloat16_t_to_svbfloat16_t(gnu_bfloat16_t x) { return x; } gnu_bfloat16_t to_gnu_bfloat16_t_from_fixed_bfloat16_t(fixed_bfloat16_t x) { return x; } fixed_bfloat16_t from_gnu_bfloat16_t_to_fixed_bfloat16_t(gnu_bfloat16_t x) { return x; }
svbool_t to_svbool_t_from_fixed(fixed_bool_t x) { return x; } fixed_bool_t from_svbool_t_to_fixed(svbool_t x) { return x; }


fixed_bool_t to_fixed_bool_t__from_svint32_t(svint32_t x) { return x; }
# 284 "Sema/attr-arm-sve-vector-bits.c"
fixed_bool_t to_fixed_bool_t__from_svuint8_t(svuint8_t x) { return x; }




svint32_t __attribute__((overloadable)) svfunc(svint32_t op1, svint32_t op2);
svfloat64_t __attribute__((overloadable)) svfunc(svfloat64_t op1, svfloat64_t op2);
svbool_t __attribute__((overloadable)) svfunc(svbool_t op1, svbool_t op2);
# 307 "Sema/attr-arm-sve-vector-bits.c"
fixed_int32_t call_int32_ff(fixed_int32_t op1, fixed_int32_t op2) { return svfunc(op1, op2); } fixed_int32_t call_int32_fs(fixed_int32_t op1, svint32_t op2) { return svfunc(op1, op2); } fixed_int32_t call_int32_sf(svint32_t op1, fixed_int32_t op2) { return svfunc(op1, op2); }
fixed_float64_t call_float64_ff(fixed_float64_t op1, fixed_float64_t op2) { return svfunc(op1, op2); } fixed_float64_t call_float64_fs(fixed_float64_t op1, svfloat64_t op2) { return svfunc(op1, op2); } fixed_float64_t call_float64_sf(svfloat64_t op1, fixed_float64_t op2) { return svfunc(op1, op2); }
fixed_bool_t call_bool_ff(fixed_bool_t op1, fixed_bool_t op2) { return svfunc(op1, op2); } fixed_bool_t call_bool_fs(fixed_bool_t op1, svbool_t op2) { return svfunc(op1, op2); } fixed_bool_t call_bool_sf(svbool_t op1, fixed_bool_t op2) { return svfunc(op1, op2); }
# 373 "Sema/attr-arm-sve-vector-bits.c"
fixed_int8_t add_fixed_int8_t(fixed_int8_t op1, fixed_int8_t op2) { return op1 + op2; } fixed_int8_t compoundadd_fixed_int8_t(fixed_int8_t op1, fixed_int8_t op2) { op1 += op2; return op1; } fixed_int8_t sub_fixed_int8_t(fixed_int8_t op1, fixed_int8_t op2) { return op1 - op2; } fixed_int8_t compoundsub_fixed_int8_t(fixed_int8_t op1, fixed_int8_t op2) { op1 -= op2; return op1; } fixed_int8_t mul_fixed_int8_t(fixed_int8_t op1, fixed_int8_t op2) { return op1 * op2; } fixed_int8_t compoundmul_fixed_int8_t(fixed_int8_t op1, fixed_int8_t op2) { op1 *= op2; return op1; } fixed_int8_t div_fixed_int8_t(fixed_int8_t op1, fixed_int8_t op2) { return op1 / op2; } fixed_int8_t compounddiv_fixed_int8_t(fixed_int8_t op1, fixed_int8_t op2) { op1 /= op2; return op1; } fixed_int8_t eq_fixed_int8_t(fixed_int8_t op1, fixed_int8_t op2) { return op1 == op2; } fixed_int8_t ne_fixed_int8_t(fixed_int8_t op1, fixed_int8_t op2) { return op1 != op2; } fixed_int8_t lt_fixed_int8_t(fixed_int8_t op1, fixed_int8_t op2) { return op1 < op2; } fixed_int8_t gt_fixed_int8_t(fixed_int8_t op1, fixed_int8_t op2) { return op1 > op2; } fixed_int8_t lte_fixed_int8_t(fixed_int8_t op1, fixed_int8_t op2) { return op1 <= op2; } fixed_int8_t gte_fixed_int8_t(fixed_int8_t op1, fixed_int8_t op2) { return op1 >= op2; } fixed_int8_t nop_fixed_int8_t(fixed_int8_t op1) { return + op1; } fixed_int8_t neg_fixed_int8_t(fixed_int8_t op1) { return - op1; } fixed_int8_t mod_fixed_int8_t(fixed_int8_t op1, fixed_int8_t op2) { return op1 % op2; } fixed_int8_t compoundmod_fixed_int8_t(fixed_int8_t op1, fixed_int8_t op2) { op1 %= op2; return op1; } fixed_int8_t and_fixed_int8_t(fixed_int8_t op1, fixed_int8_t op2) { return op1 & op2; } fixed_int8_t compoundand_fixed_int8_t(fixed_int8_t op1, fixed_int8_t op2) { op1 &= op2; return op1; } fixed_int8_t or_fixed_int8_t(fixed_int8_t op1, fixed_int8_t op2) { return op1 | op2; } fixed_int8_t compoundor_fixed_int8_t(fixed_int8_t op1, fixed_int8_t op2) { op1 |= op2; return op1; } fixed_int8_t xor_fixed_int8_t(fixed_int8_t op1, fixed_int8_t op2) { return op1 ^ op2; } fixed_int8_t compoundxor_fixed_int8_t(fixed_int8_t op1, fixed_int8_t op2) { op1 ^= op2; return op1; } fixed_int8_t shl_fixed_int8_t(fixed_int8_t op1, fixed_int8_t op2) { return op1 << op2; } fixed_int8_t compoundshl_fixed_int8_t(fixed_int8_t op1, fixed_int8_t op2) { op1 <<= op2; return op1; } fixed_int8_t shr_fixed_int8_t(fixed_int8_t op1, fixed_int8_t op2) { return op1 << op2; } fixed_int8_t compoundshr_fixed_int8_t(fixed_int8_t op1, fixed_int8_t op2) { op1 <<= op2; return op1; } fixed_int8_t not_fixed_int8_t(fixed_int8_t op1) { return ~ op1; }
fixed_int16_t add_fixed_int16_t(fixed_int16_t op1, fixed_int16_t op2) { return op1 + op2; } fixed_int16_t compoundadd_fixed_int16_t(fixed_int16_t op1, fixed_int16_t op2) { op1 += op2; return op1; } fixed_int16_t sub_fixed_int16_t(fixed_int16_t op1, fixed_int16_t op2) { return op1 - op2; } fixed_int16_t compoundsub_fixed_int16_t(fixed_int16_t op1, fixed_int16_t op2) { op1 -= op2; return op1; } fixed_int16_t mul_fixed_int16_t(fixed_int16_t op1, fixed_int16_t op2) { return op1 * op2; } fixed_int16_t compoundmul_fixed_int16_t(fixed_int16_t op1, fixed_int16_t op2) { op1 *= op2; return op1; } fixed_int16_t div_fixed_int16_t(fixed_int16_t op1, fixed_int16_t op2) { return op1 / op2; } fixed_int16_t compounddiv_fixed_int16_t(fixed_int16_t op1, fixed_int16_t op2) { op1 /= op2; return op1; } fixed_int16_t eq_fixed_int16_t(fixed_int16_t op1, fixed_int16_t op2) { return op1 == op2; } fixed_int16_t ne_fixed_int16_t(fixed_int16_t op1, fixed_int16_t op2) { return op1 != op2; } fixed_int16_t lt_fixed_int16_t(fixed_int16_t op1, fixed_int16_t op2) { return op1 < op2; } fixed_int16_t gt_fixed_int16_t(fixed_int16_t op1, fixed_int16_t op2) { return op1 > op2; } fixed_int16_t lte_fixed_int16_t(fixed_int16_t op1, fixed_int16_t op2) { return op1 <= op2; } fixed_int16_t gte_fixed_int16_t(fixed_int16_t op1, fixed_int16_t op2) { return op1 >= op2; } fixed_int16_t nop_fixed_int16_t(fixed_int16_t op1) { return + op1; } fixed_int16_t neg_fixed_int16_t(fixed_int16_t op1) { return - op1; } fixed_int16_t mod_fixed_int16_t(fixed_int16_t op1, fixed_int16_t op2) { return op1 % op2; } fixed_int16_t compoundmod_fixed_int16_t(fixed_int16_t op1, fixed_int16_t op2) { op1 %= op2; return op1; } fixed_int16_t and_fixed_int16_t(fixed_int16_t op1, fixed_int16_t op2) { return op1 & op2; } fixed_int16_t compoundand_fixed_int16_t(fixed_int16_t op1, fixed_int16_t op2) { op1 &= op2; return op1; } fixed_int16_t or_fixed_int16_t(fixed_int16_t op1, fixed_int16_t op2) { return op1 | op2; } fixed_int16_t compoundor_fixed_int16_t(fixed_int16_t op1, fixed_int16_t op2) { op1 |= op2; return op1; } fixed_int16_t xor_fixed_int16_t(fixed_int16_t op1, fixed_int16_t op2) { return op1 ^ op2; } fixed_int16_t compoundxor_fixed_int16_t(fixed_int16_t op1, fixed_int16_t op2) { op1 ^= op2; return op1; } fixed_int16_t shl_fixed_int16_t(fixed_int16_t op1, fixed_int16_t op2) { return op1 << op2; } fixed_int16_t compoundshl_fixed_int16_t(fixed_int16_t op1, fixed_int16_t op2) { op1 <<= op2; return op1; } fixed_int16_t shr_fixed_int16_t(fixed_int16_t op1, fixed_int16_t op2) { return op1 << op2; } fixed_int16_t compoundshr_fixed_int16_t(fixed_int16_t op1, fixed_int16_t op2) { op1 <<= op2; return op1; } fixed_int16_t not_fixed_int16_t(fixed_int16_t op1) { return ~ op1; }
fixed_int32_t add_fixed_int32_t(fixed_int32_t op1, fixed_int32_t op2) { return op1 + op2; } fixed_int32_t compoundadd_fixed_int32_t(fixed_int32_t op1, fixed_int32_t op2) { op1 += op2; return op1; } fixed_int32_t sub_fixed_int32_t(fixed_int32_t op1, fixed_int32_t op2) { return op1 - op2; } fixed_int32_t compoundsub_fixed_int32_t(fixed_int32_t op1, fixed_int32_t op2) { op1 -= op2; return op1; } fixed_int32_t mul_fixed_int32_t(fixed_int32_t op1, fixed_int32_t op2) { return op1 * op2; } fixed_int32_t compoundmul_fixed_int32_t(fixed_int32_t op1, fixed_int32_t op2) { op1 *= op2; return op1; } fixed_int32_t div_fixed_int32_t(fixed_int32_t op1, fixed_int32_t op2) { return op1 / op2; } fixed_int32_t compounddiv_fixed_int32_t(fixed_int32_t op1, fixed_int32_t op2) { op1 /= op2; return op1; } fixed_int32_t eq_fixed_int32_t(fixed_int32_t op1, fixed_int32_t op2) { return op1 == op2; } fixed_int32_t ne_fixed_int32_t(fixed_int32_t op1, fixed_int32_t op2) { return op1 != op2; } fixed_int32_t lt_fixed_int32_t(fixed_int32_t op1, fixed_int32_t op2) { return op1 < op2; } fixed_int32_t gt_fixed_int32_t(fixed_int32_t op1, fixed_int32_t op2) { return op1 > op2; } fixed_int32_t lte_fixed_int32_t(fixed_int32_t op1, fixed_int32_t op2) { return op1 <= op2; } fixed_int32_t gte_fixed_int32_t(fixed_int32_t op1, fixed_int32_t op2) { return op1 >= op2; } fixed_int32_t nop_fixed_int32_t(fixed_int32_t op1) { return + op1; } fixed_int32_t neg_fixed_int32_t(fixed_int32_t op1) { return - op1; } fixed_int32_t mod_fixed_int32_t(fixed_int32_t op1, fixed_int32_t op2) { return op1 % op2; } fixed_int32_t compoundmod_fixed_int32_t(fixed_int32_t op1, fixed_int32_t op2) { op1 %= op2; return op1; } fixed_int32_t and_fixed_int32_t(fixed_int32_t op1, fixed_int32_t op2) { return op1 & op2; } fixed_int32_t compoundand_fixed_int32_t(fixed_int32_t op1, fixed_int32_t op2) { op1 &= op2; return op1; } fixed_int32_t or_fixed_int32_t(fixed_int32_t op1, fixed_int32_t op2) { return op1 | op2; } fixed_int32_t compoundor_fixed_int32_t(fixed_int32_t op1, fixed_int32_t op2) { op1 |= op2; return op1; } fixed_int32_t xor_fixed_int32_t(fixed_int32_t op1, fixed_int32_t op2) { return op1 ^ op2; } fixed_int32_t compoundxor_fixed_int32_t(fixed_int32_t op1, fixed_int32_t op2) { op1 ^= op2; return op1; } fixed_int32_t shl_fixed_int32_t(fixed_int32_t op1, fixed_int32_t op2) { return op1 << op2; } fixed_int32_t compoundshl_fixed_int32_t(fixed_int32_t op1, fixed_int32_t op2) { op1 <<= op2; return op1; } fixed_int32_t shr_fixed_int32_t(fixed_int32_t op1, fixed_int32_t op2) { return op1 << op2; } fixed_int32_t compoundshr_fixed_int32_t(fixed_int32_t op1, fixed_int32_t op2) { op1 <<= op2; return op1; } fixed_int32_t not_fixed_int32_t(fixed_int32_t op1) { return ~ op1; }
fixed_int64_t add_fixed_int64_t(fixed_int64_t op1, fixed_int64_t op2) { return op1 + op2; } fixed_int64_t compoundadd_fixed_int64_t(fixed_int64_t op1, fixed_int64_t op2) { op1 += op2; return op1; } fixed_int64_t sub_fixed_int64_t(fixed_int64_t op1, fixed_int64_t op2) { return op1 - op2; } fixed_int64_t compoundsub_fixed_int64_t(fixed_int64_t op1, fixed_int64_t op2) { op1 -= op2; return op1; } fixed_int64_t mul_fixed_int64_t(fixed_int64_t op1, fixed_int64_t op2) { return op1 * op2; } fixed_int64_t compoundmul_fixed_int64_t(fixed_int64_t op1, fixed_int64_t op2) { op1 *= op2; return op1; } fixed_int64_t div_fixed_int64_t(fixed_int64_t op1, fixed_int64_t op2) { return op1 / op2; } fixed_int64_t compounddiv_fixed_int64_t(fixed_int64_t op1, fixed_int64_t op2) { op1 /= op2; return op1; } fixed_int64_t eq_fixed_int64_t(fixed_int64_t op1, fixed_int64_t op2) { return op1 == op2; } fixed_int64_t ne_fixed_int64_t(fixed_int64_t op1, fixed_int64_t op2) { return op1 != op2; } fixed_int64_t lt_fixed_int64_t(fixed_int64_t op1, fixed_int64_t op2) { return op1 < op2; } fixed_int64_t gt_fixed_int64_t(fixed_int64_t op1, fixed_int64_t op2) { return op1 > op2; } fixed_int64_t lte_fixed_int64_t(fixed_int64_t op1, fixed_int64_t op2) { return op1 <= op2; } fixed_int64_t gte_fixed_int64_t(fixed_int64_t op1, fixed_int64_t op2) { return op1 >= op2; } fixed_int64_t nop_fixed_int64_t(fixed_int64_t op1) { return + op1; } fixed_int64_t neg_fixed_int64_t(fixed_int64_t op1) { return - op1; } fixed_int64_t mod_fixed_int64_t(fixed_int64_t op1, fixed_int64_t op2) { return op1 % op2; } fixed_int64_t compoundmod_fixed_int64_t(fixed_int64_t op1, fixed_int64_t op2) { op1 %= op2; return op1; } fixed_int64_t and_fixed_int64_t(fixed_int64_t op1, fixed_int64_t op2) { return op1 & op2; } fixed_int64_t compoundand_fixed_int64_t(fixed_int64_t op1, fixed_int64_t op2) { op1 &= op2; return op1; } fixed_int64_t or_fixed_int64_t(fixed_int64_t op1, fixed_int64_t op2) { return op1 | op2; } fixed_int64_t compoundor_fixed_int64_t(fixed_int64_t op1, fixed_int64_t op2) { op1 |= op2; return op1; } fixed_int64_t xor_fixed_int64_t(fixed_int64_t op1, fixed_int64_t op2) { return op1 ^ op2; } fixed_int64_t compoundxor_fixed_int64_t(fixed_int64_t op1, fixed_int64_t op2) { op1 ^= op2; return op1; } fixed_int64_t shl_fixed_int64_t(fixed_int64_t op1, fixed_int64_t op2) { return op1 << op2; } fixed_int64_t compoundshl_fixed_int64_t(fixed_int64_t op1, fixed_int64_t op2) { op1 <<= op2; return op1; } fixed_int64_t shr_fixed_int64_t(fixed_int64_t op1, fixed_int64_t op2) { return op1 << op2; } fixed_int64_t compoundshr_fixed_int64_t(fixed_int64_t op1, fixed_int64_t op2) { op1 <<= op2; return op1; } fixed_int64_t not_fixed_int64_t(fixed_int64_t op1) { return ~ op1; }
fixed_uint8_t add_fixed_uint8_t(fixed_uint8_t op1, fixed_uint8_t op2) { return op1 + op2; } fixed_uint8_t compoundadd_fixed_uint8_t(fixed_uint8_t op1, fixed_uint8_t op2) { op1 += op2; return op1; } fixed_uint8_t sub_fixed_uint8_t(fixed_uint8_t op1, fixed_uint8_t op2) { return op1 - op2; } fixed_uint8_t compoundsub_fixed_uint8_t(fixed_uint8_t op1, fixed_uint8_t op2) { op1 -= op2; return op1; } fixed_uint8_t mul_fixed_uint8_t(fixed_uint8_t op1, fixed_uint8_t op2) { return op1 * op2; } fixed_uint8_t compoundmul_fixed_uint8_t(fixed_uint8_t op1, fixed_uint8_t op2) { op1 *= op2; return op1; } fixed_uint8_t div_fixed_uint8_t(fixed_uint8_t op1, fixed_uint8_t op2) { return op1 / op2; } fixed_uint8_t compounddiv_fixed_uint8_t(fixed_uint8_t op1, fixed_uint8_t op2) { op1 /= op2; return op1; } fixed_uint8_t eq_fixed_uint8_t(fixed_uint8_t op1, fixed_uint8_t op2) { return op1 == op2; } fixed_uint8_t ne_fixed_uint8_t(fixed_uint8_t op1, fixed_uint8_t op2) { return op1 != op2; } fixed_uint8_t lt_fixed_uint8_t(fixed_uint8_t op1, fixed_uint8_t op2) { return op1 < op2; } fixed_uint8_t gt_fixed_uint8_t(fixed_uint8_t op1, fixed_uint8_t op2) { return op1 > op2; } fixed_uint8_t lte_fixed_uint8_t(fixed_uint8_t op1, fixed_uint8_t op2) { return op1 <= op2; } fixed_uint8_t gte_fixed_uint8_t(fixed_uint8_t op1, fixed_uint8_t op2) { return op1 >= op2; } fixed_uint8_t nop_fixed_uint8_t(fixed_uint8_t op1) { return + op1; } fixed_uint8_t neg_fixed_uint8_t(fixed_uint8_t op1) { return - op1; } fixed_uint8_t mod_fixed_uint8_t(fixed_uint8_t op1, fixed_uint8_t op2) { return op1 % op2; } fixed_uint8_t compoundmod_fixed_uint8_t(fixed_uint8_t op1, fixed_uint8_t op2) { op1 %= op2; return op1; } fixed_uint8_t and_fixed_uint8_t(fixed_uint8_t op1, fixed_uint8_t op2) { return op1 & op2; } fixed_uint8_t compoundand_fixed_uint8_t(fixed_uint8_t op1, fixed_uint8_t op2) { op1 &= op2; return op1; } fixed_uint8_t or_fixed_uint8_t(fixed_uint8_t op1, fixed_uint8_t op2) { return op1 | op2; } fixed_uint8_t compoundor_fixed_uint8_t(fixed_uint8_t op1, fixed_uint8_t op2) { op1 |= op2; return op1; } fixed_uint8_t xor_fixed_uint8_t(fixed_uint8_t op1, fixed_uint8_t op2) { return op1 ^ op2; } fixed_uint8_t compoundxor_fixed_uint8_t(fixed_uint8_t op1, fixed_uint8_t op2) { op1 ^= op2; return op1; } fixed_uint8_t shl_fixed_uint8_t(fixed_uint8_t op1, fixed_uint8_t op2) { return op1 << op2; } fixed_uint8_t compoundshl_fixed_uint8_t(fixed_uint8_t op1, fixed_uint8_t op2) { op1 <<= op2; return op1; } fixed_uint8_t shr_fixed_uint8_t(fixed_uint8_t op1, fixed_uint8_t op2) { return op1 << op2; } fixed_uint8_t compoundshr_fixed_uint8_t(fixed_uint8_t op1, fixed_uint8_t op2) { op1 <<= op2; return op1; } fixed_uint8_t not_fixed_uint8_t(fixed_uint8_t op1) { return ~ op1; }
fixed_uint16_t add_fixed_uint16_t(fixed_uint16_t op1, fixed_uint16_t op2) { return op1 + op2; } fixed_uint16_t compoundadd_fixed_uint16_t(fixed_uint16_t op1, fixed_uint16_t op2) { op1 += op2; return op1; } fixed_uint16_t sub_fixed_uint16_t(fixed_uint16_t op1, fixed_uint16_t op2) { return op1 - op2; } fixed_uint16_t compoundsub_fixed_uint16_t(fixed_uint16_t op1, fixed_uint16_t op2) { op1 -= op2; return op1; } fixed_uint16_t mul_fixed_uint16_t(fixed_uint16_t op1, fixed_uint16_t op2) { return op1 * op2; } fixed_uint16_t compoundmul_fixed_uint16_t(fixed_uint16_t op1, fixed_uint16_t op2) { op1 *= op2; return op1; } fixed_uint16_t div_fixed_uint16_t(fixed_uint16_t op1, fixed_uint16_t op2) { return op1 / op2; } fixed_uint16_t compounddiv_fixed_uint16_t(fixed_uint16_t op1, fixed_uint16_t op2) { op1 /= op2; return op1; } fixed_uint16_t eq_fixed_uint16_t(fixed_uint16_t op1, fixed_uint16_t op2) { return op1 == op2; } fixed_uint16_t ne_fixed_uint16_t(fixed_uint16_t op1, fixed_uint16_t op2) { return op1 != op2; } fixed_uint16_t lt_fixed_uint16_t(fixed_uint16_t op1, fixed_uint16_t op2) { return op1 < op2; } fixed_uint16_t gt_fixed_uint16_t(fixed_uint16_t op1, fixed_uint16_t op2) { return op1 > op2; } fixed_uint16_t lte_fixed_uint16_t(fixed_uint16_t op1, fixed_uint16_t op2) { return op1 <= op2; } fixed_uint16_t gte_fixed_uint16_t(fixed_uint16_t op1, fixed_uint16_t op2) { return op1 >= op2; } fixed_uint16_t nop_fixed_uint16_t(fixed_uint16_t op1) { return + op1; } fixed_uint16_t neg_fixed_uint16_t(fixed_uint16_t op1) { return - op1; } fixed_uint16_t mod_fixed_uint16_t(fixed_uint16_t op1, fixed_uint16_t op2) { return op1 % op2; } fixed_uint16_t compoundmod_fixed_uint16_t(fixed_uint16_t op1, fixed_uint16_t op2) { op1 %= op2; return op1; } fixed_uint16_t and_fixed_uint16_t(fixed_uint16_t op1, fixed_uint16_t op2) { return op1 & op2; } fixed_uint16_t compoundand_fixed_uint16_t(fixed_uint16_t op1, fixed_uint16_t op2) { op1 &= op2; return op1; } fixed_uint16_t or_fixed_uint16_t(fixed_uint16_t op1, fixed_uint16_t op2) { return op1 | op2; } fixed_uint16_t compoundor_fixed_uint16_t(fixed_uint16_t op1, fixed_uint16_t op2) { op1 |= op2; return op1; } fixed_uint16_t xor_fixed_uint16_t(fixed_uint16_t op1, fixed_uint16_t op2) { return op1 ^ op2; } fixed_uint16_t compoundxor_fixed_uint16_t(fixed_uint16_t op1, fixed_uint16_t op2) { op1 ^= op2; return op1; } fixed_uint16_t shl_fixed_uint16_t(fixed_uint16_t op1, fixed_uint16_t op2) { return op1 << op2; } fixed_uint16_t compoundshl_fixed_uint16_t(fixed_uint16_t op1, fixed_uint16_t op2) { op1 <<= op2; return op1; } fixed_uint16_t shr_fixed_uint16_t(fixed_uint16_t op1, fixed_uint16_t op2) { return op1 << op2; } fixed_uint16_t compoundshr_fixed_uint16_t(fixed_uint16_t op1, fixed_uint16_t op2) { op1 <<= op2; return op1; } fixed_uint16_t not_fixed_uint16_t(fixed_uint16_t op1) { return ~ op1; }
fixed_uint32_t add_fixed_uint32_t(fixed_uint32_t op1, fixed_uint32_t op2) { return op1 + op2; } fixed_uint32_t compoundadd_fixed_uint32_t(fixed_uint32_t op1, fixed_uint32_t op2) { op1 += op2; return op1; } fixed_uint32_t sub_fixed_uint32_t(fixed_uint32_t op1, fixed_uint32_t op2) { return op1 - op2; } fixed_uint32_t compoundsub_fixed_uint32_t(fixed_uint32_t op1, fixed_uint32_t op2) { op1 -= op2; return op1; } fixed_uint32_t mul_fixed_uint32_t(fixed_uint32_t op1, fixed_uint32_t op2) { return op1 * op2; } fixed_uint32_t compoundmul_fixed_uint32_t(fixed_uint32_t op1, fixed_uint32_t op2) { op1 *= op2; return op1; } fixed_uint32_t div_fixed_uint32_t(fixed_uint32_t op1, fixed_uint32_t op2) { return op1 / op2; } fixed_uint32_t compounddiv_fixed_uint32_t(fixed_uint32_t op1, fixed_uint32_t op2) { op1 /= op2; return op1; } fixed_uint32_t eq_fixed_uint32_t(fixed_uint32_t op1, fixed_uint32_t op2) { return op1 == op2; } fixed_uint32_t ne_fixed_uint32_t(fixed_uint32_t op1, fixed_uint32_t op2) { return op1 != op2; } fixed_uint32_t lt_fixed_uint32_t(fixed_uint32_t op1, fixed_uint32_t op2) { return op1 < op2; } fixed_uint32_t gt_fixed_uint32_t(fixed_uint32_t op1, fixed_uint32_t op2) { return op1 > op2; } fixed_uint32_t lte_fixed_uint32_t(fixed_uint32_t op1, fixed_uint32_t op2) { return op1 <= op2; } fixed_uint32_t gte_fixed_uint32_t(fixed_uint32_t op1, fixed_uint32_t op2) { return op1 >= op2; } fixed_uint32_t nop_fixed_uint32_t(fixed_uint32_t op1) { return + op1; } fixed_uint32_t neg_fixed_uint32_t(fixed_uint32_t op1) { return - op1; } fixed_uint32_t mod_fixed_uint32_t(fixed_uint32_t op1, fixed_uint32_t op2) { return op1 % op2; } fixed_uint32_t compoundmod_fixed_uint32_t(fixed_uint32_t op1, fixed_uint32_t op2) { op1 %= op2; return op1; } fixed_uint32_t and_fixed_uint32_t(fixed_uint32_t op1, fixed_uint32_t op2) { return op1 & op2; } fixed_uint32_t compoundand_fixed_uint32_t(fixed_uint32_t op1, fixed_uint32_t op2) { op1 &= op2; return op1; } fixed_uint32_t or_fixed_uint32_t(fixed_uint32_t op1, fixed_uint32_t op2) { return op1 | op2; } fixed_uint32_t compoundor_fixed_uint32_t(fixed_uint32_t op1, fixed_uint32_t op2) { op1 |= op2; return op1; } fixed_uint32_t xor_fixed_uint32_t(fixed_uint32_t op1, fixed_uint32_t op2) { return op1 ^ op2; } fixed_uint32_t compoundxor_fixed_uint32_t(fixed_uint32_t op1, fixed_uint32_t op2) { op1 ^= op2; return op1; } fixed_uint32_t shl_fixed_uint32_t(fixed_uint32_t op1, fixed_uint32_t op2) { return op1 << op2; } fixed_uint32_t compoundshl_fixed_uint32_t(fixed_uint32_t op1, fixed_uint32_t op2) { op1 <<= op2; return op1; } fixed_uint32_t shr_fixed_uint32_t(fixed_uint32_t op1, fixed_uint32_t op2) { return op1 << op2; } fixed_uint32_t compoundshr_fixed_uint32_t(fixed_uint32_t op1, fixed_uint32_t op2) { op1 <<= op2; return op1; } fixed_uint32_t not_fixed_uint32_t(fixed_uint32_t op1) { return ~ op1; }
fixed_uint64_t add_fixed_uint64_t(fixed_uint64_t op1, fixed_uint64_t op2) { return op1 + op2; } fixed_uint64_t compoundadd_fixed_uint64_t(fixed_uint64_t op1, fixed_uint64_t op2) { op1 += op2; return op1; } fixed_uint64_t sub_fixed_uint64_t(fixed_uint64_t op1, fixed_uint64_t op2) { return op1 - op2; } fixed_uint64_t compoundsub_fixed_uint64_t(fixed_uint64_t op1, fixed_uint64_t op2) { op1 -= op2; return op1; } fixed_uint64_t mul_fixed_uint64_t(fixed_uint64_t op1, fixed_uint64_t op2) { return op1 * op2; } fixed_uint64_t compoundmul_fixed_uint64_t(fixed_uint64_t op1, fixed_uint64_t op2) { op1 *= op2; return op1; } fixed_uint64_t div_fixed_uint64_t(fixed_uint64_t op1, fixed_uint64_t op2) { return op1 / op2; } fixed_uint64_t compounddiv_fixed_uint64_t(fixed_uint64_t op1, fixed_uint64_t op2) { op1 /= op2; return op1; } fixed_uint64_t eq_fixed_uint64_t(fixed_uint64_t op1, fixed_uint64_t op2) { return op1 == op2; } fixed_uint64_t ne_fixed_uint64_t(fixed_uint64_t op1, fixed_uint64_t op2) { return op1 != op2; } fixed_uint64_t lt_fixed_uint64_t(fixed_uint64_t op1, fixed_uint64_t op2) { return op1 < op2; } fixed_uint64_t gt_fixed_uint64_t(fixed_uint64_t op1, fixed_uint64_t op2) { return op1 > op2; } fixed_uint64_t lte_fixed_uint64_t(fixed_uint64_t op1, fixed_uint64_t op2) { return op1 <= op2; } fixed_uint64_t gte_fixed_uint64_t(fixed_uint64_t op1, fixed_uint64_t op2) { return op1 >= op2; } fixed_uint64_t nop_fixed_uint64_t(fixed_uint64_t op1) { return + op1; } fixed_uint64_t neg_fixed_uint64_t(fixed_uint64_t op1) { return - op1; } fixed_uint64_t mod_fixed_uint64_t(fixed_uint64_t op1, fixed_uint64_t op2) { return op1 % op2; } fixed_uint64_t compoundmod_fixed_uint64_t(fixed_uint64_t op1, fixed_uint64_t op2) { op1 %= op2; return op1; } fixed_uint64_t and_fixed_uint64_t(fixed_uint64_t op1, fixed_uint64_t op2) { return op1 & op2; } fixed_uint64_t compoundand_fixed_uint64_t(fixed_uint64_t op1, fixed_uint64_t op2) { op1 &= op2; return op1; } fixed_uint64_t or_fixed_uint64_t(fixed_uint64_t op1, fixed_uint64_t op2) { return op1 | op2; } fixed_uint64_t compoundor_fixed_uint64_t(fixed_uint64_t op1, fixed_uint64_t op2) { op1 |= op2; return op1; } fixed_uint64_t xor_fixed_uint64_t(fixed_uint64_t op1, fixed_uint64_t op2) { return op1 ^ op2; } fixed_uint64_t compoundxor_fixed_uint64_t(fixed_uint64_t op1, fixed_uint64_t op2) { op1 ^= op2; return op1; } fixed_uint64_t shl_fixed_uint64_t(fixed_uint64_t op1, fixed_uint64_t op2) { return op1 << op2; } fixed_uint64_t compoundshl_fixed_uint64_t(fixed_uint64_t op1, fixed_uint64_t op2) { op1 <<= op2; return op1; } fixed_uint64_t shr_fixed_uint64_t(fixed_uint64_t op1, fixed_uint64_t op2) { return op1 << op2; } fixed_uint64_t compoundshr_fixed_uint64_t(fixed_uint64_t op1, fixed_uint64_t op2) { op1 <<= op2; return op1; } fixed_uint64_t not_fixed_uint64_t(fixed_uint64_t op1) { return ~ op1; }

fixed_float16_t add_fixed_float16_t(fixed_float16_t op1, fixed_float16_t op2) { return op1 + op2; } fixed_float16_t compoundadd_fixed_float16_t(fixed_float16_t op1, fixed_float16_t op2) { op1 += op2; return op1; } fixed_float16_t sub_fixed_float16_t(fixed_float16_t op1, fixed_float16_t op2) { return op1 - op2; } fixed_float16_t compoundsub_fixed_float16_t(fixed_float16_t op1, fixed_float16_t op2) { op1 -= op2; return op1; } fixed_float16_t mul_fixed_float16_t(fixed_float16_t op1, fixed_float16_t op2) { return op1 * op2; } fixed_float16_t compoundmul_fixed_float16_t(fixed_float16_t op1, fixed_float16_t op2) { op1 *= op2; return op1; } fixed_float16_t div_fixed_float16_t(fixed_float16_t op1, fixed_float16_t op2) { return op1 / op2; } fixed_float16_t compounddiv_fixed_float16_t(fixed_float16_t op1, fixed_float16_t op2) { op1 /= op2; return op1; } fixed_float16_t eq_fixed_float16_t(fixed_float16_t op1, fixed_float16_t op2) { return op1 == op2; } fixed_float16_t ne_fixed_float16_t(fixed_float16_t op1, fixed_float16_t op2) { return op1 != op2; } fixed_float16_t lt_fixed_float16_t(fixed_float16_t op1, fixed_float16_t op2) { return op1 < op2; } fixed_float16_t gt_fixed_float16_t(fixed_float16_t op1, fixed_float16_t op2) { return op1 > op2; } fixed_float16_t lte_fixed_float16_t(fixed_float16_t op1, fixed_float16_t op2) { return op1 <= op2; } fixed_float16_t gte_fixed_float16_t(fixed_float16_t op1, fixed_float16_t op2) { return op1 >= op2; } fixed_float16_t nop_fixed_float16_t(fixed_float16_t op1) { return + op1; } fixed_float16_t neg_fixed_float16_t(fixed_float16_t op1) { return - op1; }
fixed_float32_t add_fixed_float32_t(fixed_float32_t op1, fixed_float32_t op2) { return op1 + op2; } fixed_float32_t compoundadd_fixed_float32_t(fixed_float32_t op1, fixed_float32_t op2) { op1 += op2; return op1; } fixed_float32_t sub_fixed_float32_t(fixed_float32_t op1, fixed_float32_t op2) { return op1 - op2; } fixed_float32_t compoundsub_fixed_float32_t(fixed_float32_t op1, fixed_float32_t op2) { op1 -= op2; return op1; } fixed_float32_t mul_fixed_float32_t(fixed_float32_t op1, fixed_float32_t op2) { return op1 * op2; } fixed_float32_t compoundmul_fixed_float32_t(fixed_float32_t op1, fixed_float32_t op2) { op1 *= op2; return op1; } fixed_float32_t div_fixed_float32_t(fixed_float32_t op1, fixed_float32_t op2) { return op1 / op2; } fixed_float32_t compounddiv_fixed_float32_t(fixed_float32_t op1, fixed_float32_t op2) { op1 /= op2; return op1; } fixed_float32_t eq_fixed_float32_t(fixed_float32_t op1, fixed_float32_t op2) { return op1 == op2; } fixed_float32_t ne_fixed_float32_t(fixed_float32_t op1, fixed_float32_t op2) { return op1 != op2; } fixed_float32_t lt_fixed_float32_t(fixed_float32_t op1, fixed_float32_t op2) { return op1 < op2; } fixed_float32_t gt_fixed_float32_t(fixed_float32_t op1, fixed_float32_t op2) { return op1 > op2; } fixed_float32_t lte_fixed_float32_t(fixed_float32_t op1, fixed_float32_t op2) { return op1 <= op2; } fixed_float32_t gte_fixed_float32_t(fixed_float32_t op1, fixed_float32_t op2) { return op1 >= op2; } fixed_float32_t nop_fixed_float32_t(fixed_float32_t op1) { return + op1; } fixed_float32_t neg_fixed_float32_t(fixed_float32_t op1) { return - op1; }
fixed_float64_t add_fixed_float64_t(fixed_float64_t op1, fixed_float64_t op2) { return op1 + op2; } fixed_float64_t compoundadd_fixed_float64_t(fixed_float64_t op1, fixed_float64_t op2) { op1 += op2; return op1; } fixed_float64_t sub_fixed_float64_t(fixed_float64_t op1, fixed_float64_t op2) { return op1 - op2; } fixed_float64_t compoundsub_fixed_float64_t(fixed_float64_t op1, fixed_float64_t op2) { op1 -= op2; return op1; } fixed_float64_t mul_fixed_float64_t(fixed_float64_t op1, fixed_float64_t op2) { return op1 * op2; } fixed_float64_t compoundmul_fixed_float64_t(fixed_float64_t op1, fixed_float64_t op2) { op1 *= op2; return op1; } fixed_float64_t div_fixed_float64_t(fixed_float64_t op1, fixed_float64_t op2) { return op1 / op2; } fixed_float64_t compounddiv_fixed_float64_t(fixed_float64_t op1, fixed_float64_t op2) { op1 /= op2; return op1; } fixed_float64_t eq_fixed_float64_t(fixed_float64_t op1, fixed_float64_t op2) { return op1 == op2; } fixed_float64_t ne_fixed_float64_t(fixed_float64_t op1, fixed_float64_t op2) { return op1 != op2; } fixed_float64_t lt_fixed_float64_t(fixed_float64_t op1, fixed_float64_t op2) { return op1 < op2; } fixed_float64_t gt_fixed_float64_t(fixed_float64_t op1, fixed_float64_t op2) { return op1 > op2; } fixed_float64_t lte_fixed_float64_t(fixed_float64_t op1, fixed_float64_t op2) { return op1 <= op2; } fixed_float64_t gte_fixed_float64_t(fixed_float64_t op1, fixed_float64_t op2) { return op1 >= op2; } fixed_float64_t nop_fixed_float64_t(fixed_float64_t op1) { return + op1; } fixed_float64_t neg_fixed_float64_t(fixed_float64_t op1) { return - op1; }



__arm_locally_streaming void locally_streaming() {
  svint8_t t1 = extern_int8;
  svbool_t t2 = extern_bool;
  void* t3 = extern_int8_ptr;
}
void streaming(void) __arm_streaming {
  svint8_t t1 = extern_int8;
  svbool_t t2 = extern_bool;
  void* t3 = extern_int8_ptr;
}
void streaming_compatible(void) __arm_streaming_compatible {
  svint8_t t1 = extern_int8;

  svbool_t t2 = extern_bool;

  void* t3 = extern_int8_ptr;
}
__arm_locally_streaming void locally_streaming_arg(fixed_int8_t x) {}
