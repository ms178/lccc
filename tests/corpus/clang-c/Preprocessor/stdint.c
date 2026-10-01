// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
# 1 "Preprocessor/stdint.c"
# 1 "<built-in>" 1
# 1 "<built-in>" 3
# 412 "<built-in>" 3
# 1 "<command line>" 1
# 1 "<built-in>" 2
# 1 "Preprocessor/stdint.c" 2
# 1652 "Preprocessor/stdint.c"
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
# 1653 "Preprocessor/stdint.c" 2

INT8_MAX_ (127)
INT8_MIN_ (-128)
UINT8_MAX_ (255)
INT_LEAST8_MIN_ (-128)
INT_LEAST8_MAX_ (127)
UINT_LEAST8_MAX_ (255)
INT_FAST8_MIN_ (-128)
INT_FAST8_MAX_ (127)
UINT_FAST8_MAX_ (255)

INT16_MAX_ (32767)
INT16_MIN_ (-32767-1)
UINT16_MAX_ (65535)
INT_LEAST16_MIN_ (-32767-1)
INT_LEAST16_MAX_ (32767)
UINT_LEAST16_MAX_ (65535)
INT_FAST16_MIN_ (-9223372036854775807L-1)
INT_FAST16_MAX_ (9223372036854775807L)
UINT_FAST16_MAX_ (18446744073709551615UL)

INT32_MAX_ (2147483647)
INT32_MIN_ (-2147483647-1)
UINT32_MAX_ (4294967295U)
INT_LEAST32_MIN_ (-2147483647-1)
INT_LEAST32_MAX_ (2147483647)
UINT_LEAST32_MAX_ (4294967295U)
INT_FAST32_MIN_ (-9223372036854775807L-1)
INT_FAST32_MAX_ (9223372036854775807L)
UINT_FAST32_MAX_ (18446744073709551615UL)

INT64_MAX_ (9223372036854775807L)
INT64_MIN_ (-9223372036854775807L -1)
UINT64_MAX_ (18446744073709551615UL)
INT_LEAST64_MIN_ (-9223372036854775807L -1)
INT_LEAST64_MAX_ (9223372036854775807L)
UINT_LEAST64_MAX_ (18446744073709551615UL)
INT_FAST64_MIN_ (-9223372036854775807L -1)
INT_FAST64_MAX_ (9223372036854775807L)
UINT_FAST64_MAX_ (18446744073709551615UL)

INTPTR_MIN_ (-9223372036854775807L-1)
INTPTR_MAX_ (9223372036854775807L)
UINTPTR_MAX_ (18446744073709551615UL)
PTRDIFF_MIN_ (-9223372036854775807L-1)
PTRDIFF_MAX_ (9223372036854775807L)
SIZE_MAX_ (18446744073709551615UL)

INTMAX_MIN_ (-9223372036854775807L -1)
INTMAX_MAX_ (9223372036854775807L)
UINTMAX_MAX_ (18446744073709551615UL)

SIG_ATOMIC_MIN_ (-2147483647-1)
SIG_ATOMIC_MAX_ (2147483647)
WINT_MIN_ (0u)
WINT_MAX_ (4294967295u)

WCHAR_MAX_ (2147483647)
WCHAR_MIN_ (-2147483647 - 1)

INT8_C_(0) 0
UINT8_C_(0) 0
INT16_C_(0) 0
UINT16_C_(0) 0
INT32_C_(0) 0
UINT32_C_(0) 0U
INT64_C_(0) 0L
UINT64_C_(0) 0UL

INTMAX_C_(0) 0L
UINTMAX_C_(0) 0UL
