// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
# 1 "Sema/atomic-ops.c"
# 1 "<built-in>" 1
# 1 "<built-in>" 3
# 412 "<built-in>" 3
# 1 "<command line>" 1
# 1 "<built-in>" 2
# 1 "Sema/atomic-ops.c" 2
# 22 "Sema/atomic-ops.c"
# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdatomic.h" 1
# 27 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdatomic.h"
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
# 28 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdatomic.h" 2
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
# 29 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdatomic.h" 2
# 68 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdatomic.h"
typedef enum memory_order {
  memory_order_relaxed = 0,
  memory_order_consume = 1,
  memory_order_acquire = 2,
  memory_order_release = 3,
  memory_order_acq_rel = 4,
  memory_order_seq_cst = 5
} memory_order;






void atomic_thread_fence(memory_order);
void atomic_signal_fence(memory_order);
# 97 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdatomic.h"
typedef _Atomic(_Bool) atomic_bool;

typedef _Atomic(char) atomic_char;
typedef _Atomic(signed char) atomic_schar;
typedef _Atomic(unsigned char) atomic_uchar;
typedef _Atomic(short) atomic_short;
typedef _Atomic(unsigned short) atomic_ushort;
typedef _Atomic(int) atomic_int;
typedef _Atomic(unsigned int) atomic_uint;
typedef _Atomic(long) atomic_long;
typedef _Atomic(unsigned long) atomic_ulong;
typedef _Atomic(long long) atomic_llong;
typedef _Atomic(unsigned long long) atomic_ullong;



typedef _Atomic(uint_least16_t) atomic_char16_t;
typedef _Atomic(uint_least32_t) atomic_char32_t;
typedef _Atomic(wchar_t) atomic_wchar_t;
typedef _Atomic(int_least8_t) atomic_int_least8_t;
typedef _Atomic(uint_least8_t) atomic_uint_least8_t;
typedef _Atomic(int_least16_t) atomic_int_least16_t;
typedef _Atomic(uint_least16_t) atomic_uint_least16_t;
typedef _Atomic(int_least32_t) atomic_int_least32_t;
typedef _Atomic(uint_least32_t) atomic_uint_least32_t;
typedef _Atomic(int_least64_t) atomic_int_least64_t;
typedef _Atomic(uint_least64_t) atomic_uint_least64_t;
typedef _Atomic(int_fast8_t) atomic_int_fast8_t;
typedef _Atomic(uint_fast8_t) atomic_uint_fast8_t;
typedef _Atomic(int_fast16_t) atomic_int_fast16_t;
typedef _Atomic(uint_fast16_t) atomic_uint_fast16_t;
typedef _Atomic(int_fast32_t) atomic_int_fast32_t;
typedef _Atomic(uint_fast32_t) atomic_uint_fast32_t;
typedef _Atomic(int_fast64_t) atomic_int_fast64_t;
typedef _Atomic(uint_fast64_t) atomic_uint_fast64_t;
typedef _Atomic(intptr_t) atomic_intptr_t;
typedef _Atomic(uintptr_t) atomic_uintptr_t;
typedef _Atomic(size_t) atomic_size_t;
typedef _Atomic(ptrdiff_t) atomic_ptrdiff_t;
typedef _Atomic(intmax_t) atomic_intmax_t;
typedef _Atomic(uintmax_t) atomic_uintmax_t;
# 173 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdatomic.h"
typedef struct atomic_flag { atomic_bool _Value; } atomic_flag;
# 186 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdatomic.h"
_Bool atomic_flag_test_and_set(volatile atomic_flag *);
_Bool atomic_flag_test_and_set_explicit(volatile atomic_flag *, memory_order);

void atomic_flag_clear(volatile atomic_flag *);
void atomic_flag_clear_explicit(volatile atomic_flag *, memory_order);
# 23 "Sema/atomic-ops.c" 2

struct S { char c[3]; };

_Static_assert(2 == 2, "");
_Static_assert(2 == 2, "");
_Static_assert(2 == 2, "");
_Static_assert(2 == 2, "");
_Static_assert(2 == 2, "");
_Static_assert(2 == 2, "");
_Static_assert(2 == 2, "");
_Static_assert(2 == 2, "");
_Static_assert(2 == 2, "");
_Static_assert(2 == 2, "");
_Static_assert(2 == 2, "");
_Static_assert(2 == 2, "");
_Static_assert(2 == 2, "");
_Static_assert(2 == 2, "");
_Static_assert(2 == 2, "");
_Static_assert(2 == 2, "");
_Static_assert(2 == 2, "");
_Static_assert(2 == 2, "");
_Static_assert(2 == 2, "");
_Static_assert(2 == 2, "");

_Static_assert(__c11_atomic_is_lock_free(1), "");
_Static_assert(__c11_atomic_is_lock_free(2), "");
_Static_assert(__c11_atomic_is_lock_free(3), "");
_Static_assert(__c11_atomic_is_lock_free(4), "");
_Static_assert(__c11_atomic_is_lock_free(8), "");

_Static_assert(__c11_atomic_is_lock_free(16), "");



_Static_assert(__c11_atomic_is_lock_free(17), "");

_Static_assert(__atomic_is_lock_free(1, 0), "");
_Static_assert(__atomic_is_lock_free(2, 0), "");
_Static_assert(__atomic_is_lock_free(3, 0), "");
_Static_assert(__atomic_is_lock_free(4, 0), "");
_Static_assert(__atomic_is_lock_free(8, 0), "");

_Static_assert(__atomic_is_lock_free(16, 0), "");



_Static_assert(__atomic_is_lock_free(17, 0), "");

_Static_assert(__c11_atomic_is_lock_free(sizeof(*((atomic_char*)0))), "");
_Static_assert(__c11_atomic_is_lock_free(sizeof(*((atomic_short*)0))), "");
_Static_assert(__c11_atomic_is_lock_free(sizeof(*((atomic_int*)0))), "");
_Static_assert(__c11_atomic_is_lock_free(sizeof(*((atomic_long*)0))), "");


_Static_assert(__c11_atomic_is_lock_free(sizeof(*((_Atomic(__int128)*)0))), "");



_Static_assert(__c11_atomic_is_lock_free(sizeof(*(0 + (atomic_char*)0))), "");

char i8;
short i16;
int i32;
int __attribute__((vector_size(8))) i64;
struct Incomplete *incomplete;

_Static_assert(__atomic_is_lock_free(1, &i8), "");
_Static_assert(__atomic_is_lock_free(1, &i64), "");
_Static_assert(__atomic_is_lock_free(2, &i8), "");
_Static_assert(__atomic_is_lock_free(2, &i16), "");
_Static_assert(__atomic_is_lock_free(2, &i64), "");
_Static_assert(__atomic_is_lock_free(4, &i16), "");
_Static_assert(__atomic_is_lock_free(4, &i32), "");
_Static_assert(__atomic_is_lock_free(4, &i64), "");
_Static_assert(__atomic_is_lock_free(8, &i32), "");
_Static_assert(__atomic_is_lock_free(8, &i64), "");

_Static_assert(__atomic_always_lock_free(1, 0), "");
_Static_assert(__atomic_always_lock_free(2, 0), "");
_Static_assert(!__atomic_always_lock_free(3, 0), "");
_Static_assert(__atomic_always_lock_free(4, 0), "");
_Static_assert(__atomic_always_lock_free(8, 0), "");

_Static_assert(!__atomic_always_lock_free(16, 0), "");



_Static_assert(!__atomic_always_lock_free(17, 0), "");

_Static_assert(__atomic_always_lock_free(1, incomplete), "");
_Static_assert(!__atomic_always_lock_free(2, incomplete), "");
_Static_assert(!__atomic_always_lock_free(4, incomplete), "");

_Static_assert(__atomic_always_lock_free(1, &i8), "");
_Static_assert(__atomic_always_lock_free(1, &i64), "");
_Static_assert(!__atomic_always_lock_free(2, &i8), "");
_Static_assert(__atomic_always_lock_free(2, &i16), "");
_Static_assert(__atomic_always_lock_free(2, &i64), "");
_Static_assert(!__atomic_always_lock_free(4, &i16), "");
_Static_assert(__atomic_always_lock_free(4, &i32), "");
_Static_assert(__atomic_always_lock_free(4, &i64), "");
_Static_assert(!__atomic_always_lock_free(8, &i32), "");
_Static_assert(__atomic_always_lock_free(8, &i64), "");



_Static_assert(__atomic_is_lock_free(1, (void*)1), "");
_Static_assert(__atomic_is_lock_free(1, (void*)-1), "");
_Static_assert(__atomic_is_lock_free(4, (void*)2), "");
_Static_assert(__atomic_is_lock_free(4, (void*)-2), "");
_Static_assert(__atomic_is_lock_free(4, (void*)4), "");
_Static_assert(__atomic_is_lock_free(4, (void*)-4), "");

_Static_assert(__atomic_always_lock_free(1, (void*)1), "");
_Static_assert(__atomic_always_lock_free(1, (void*)-1), "");
_Static_assert(!__atomic_always_lock_free(4, (void*)2), "");
_Static_assert(!__atomic_always_lock_free(4, (void*)-2), "");
_Static_assert(__atomic_always_lock_free(4, (void*)4), "");
_Static_assert(__atomic_always_lock_free(4, (void*)-4), "");


_Static_assert(__atomic_always_lock_free(1, "string"), "");
_Static_assert(!__atomic_always_lock_free(2, "string"), "");
_Static_assert(__atomic_always_lock_free(2, (int[2]){}), "");
void dummyfn();
_Static_assert(__atomic_always_lock_free(2, dummyfn) || 1, "");






void f(_Atomic(int) *i, const _Atomic(int) *ci,
       _Atomic(int*) *p, _Atomic(float) *f, _Atomic(double) *d,
       _Atomic(long double) *ld,
       int *I, const int *CI,
       int **P, float *F, double *D, struct S *s1, struct S *s2) {
  __c11_atomic_init(I, 5);
  __c11_atomic_init(ci, 5);

  __c11_atomic_load(0);
  __c11_atomic_load(0,0,0);
  __c11_atomic_store(0,0,0);
  __c11_atomic_store((int*)0,0,0);
  __c11_atomic_store(i, 0, memory_order_relaxed);
  __c11_atomic_store(ci, 0, memory_order_relaxed);

  __c11_atomic_load(i, memory_order_seq_cst);
  __c11_atomic_load(p, memory_order_seq_cst);
  __c11_atomic_load(f, memory_order_seq_cst);
  __c11_atomic_load(ci, memory_order_seq_cst);

  int load_n_1 = __atomic_load_n(I, memory_order_relaxed);
  int *load_n_2 = __atomic_load_n(P, memory_order_relaxed);
  float load_n_3 = __atomic_load_n(D, memory_order_relaxed);
  __atomic_load_n(s1, memory_order_relaxed);
  load_n_1 = __atomic_load_n(CI, memory_order_relaxed);

  __atomic_load(i, I, memory_order_relaxed);
  __atomic_load(CI, I, memory_order_relaxed);

  __atomic_load(I, i, memory_order_relaxed);
  __atomic_load(I, *P, memory_order_relaxed);
  __atomic_load(I, *P, memory_order_relaxed, 42);
  (int)__atomic_load(I, I, memory_order_seq_cst);
  __atomic_load(s1, s2, memory_order_acquire);
  __atomic_load(CI, I, memory_order_relaxed);
  __atomic_load(I, CI, memory_order_relaxed);
  __atomic_load(CI, CI, memory_order_relaxed);

  __c11_atomic_store(i, 1, memory_order_seq_cst);
  __c11_atomic_store(p, 1, memory_order_seq_cst);
  (int)__c11_atomic_store(f, 1, memory_order_seq_cst);

  __atomic_store_n(I, 4, memory_order_release);
  __atomic_store_n(I, 4.0, memory_order_release);
  __atomic_store_n(CI, 4, memory_order_release);
  __atomic_store_n(I, P, memory_order_release);
  __atomic_store_n(i, 1, memory_order_release);
  __atomic_store_n(s1, *s2, memory_order_release);
  __atomic_store_n(I, I, memory_order_release);

  __atomic_store(I, *P, memory_order_release);
  __atomic_store(CI, I, memory_order_release);
  __atomic_store(s1, s2, memory_order_release);
  __atomic_store(i, I, memory_order_release);

  int exchange_1 = __c11_atomic_exchange(i, 1, memory_order_seq_cst);
  int exchange_2 = __c11_atomic_exchange(I, 1, memory_order_seq_cst);
  int exchange_3 = __atomic_exchange_n(i, 1, memory_order_seq_cst);
  int exchange_4 = __atomic_exchange_n(I, 1, memory_order_seq_cst);

  __atomic_exchange(s1, s2, s2, memory_order_seq_cst);
  __atomic_exchange(s1, I, P, memory_order_seq_cst);
  (int)__atomic_exchange(s1, s2, s2, memory_order_seq_cst);
  __atomic_exchange(I, I, I, memory_order_seq_cst);
  __atomic_exchange(CI, I, I, memory_order_seq_cst);
  __atomic_exchange(I, I, CI, memory_order_seq_cst);

  __c11_atomic_fetch_add(i, 1, memory_order_seq_cst);
  __c11_atomic_fetch_add(p, 1, memory_order_seq_cst);
  __c11_atomic_fetch_add(f, 1.0f, memory_order_seq_cst);
  __c11_atomic_fetch_add(d, 1.0, memory_order_seq_cst);
  __c11_atomic_fetch_add(ld, 1.0, memory_order_seq_cst);
  __c11_atomic_fetch_min(i, 1, memory_order_seq_cst);
  __c11_atomic_fetch_min(p, 1, memory_order_seq_cst);
  __c11_atomic_fetch_min(f, 1.0f, memory_order_seq_cst);
  __c11_atomic_fetch_min(d, 1.0, memory_order_seq_cst);
  __c11_atomic_fetch_min(ld, 1.0, memory_order_seq_cst);
  __c11_atomic_fetch_max(i, 1, memory_order_seq_cst);
  __c11_atomic_fetch_max(p, 1, memory_order_seq_cst);
  __c11_atomic_fetch_max(f, 1.0f, memory_order_seq_cst);
  __c11_atomic_fetch_max(d, 1.0, memory_order_seq_cst);
  __c11_atomic_fetch_max(ld, 1.0, memory_order_seq_cst);

  __atomic_fetch_add(i, 3, memory_order_seq_cst);
  __atomic_fetch_sub(I, 3, memory_order_seq_cst);
  __atomic_fetch_sub(P, 3, memory_order_seq_cst);
  __atomic_fetch_sub(F, 3, memory_order_seq_cst);
  __atomic_fetch_sub(s1, 3, memory_order_seq_cst);
  __atomic_fetch_min(F, 3, memory_order_seq_cst);
  __atomic_fetch_min(D, 3, memory_order_seq_cst);
  __atomic_fetch_max(F, 3, memory_order_seq_cst);
  __atomic_fetch_max(D, 3, memory_order_seq_cst);
  __atomic_fetch_max(P, 3, memory_order_seq_cst);
  __atomic_fetch_max(p, 3);

  __atomic_fetch_uinc(F, 1, memory_order_seq_cst);
  __atomic_fetch_udec(F, 1, memory_order_seq_cst);

  __c11_atomic_fetch_and(i, 1, memory_order_seq_cst);
  __c11_atomic_fetch_and(p, 1, memory_order_seq_cst);
  __c11_atomic_fetch_and(f, 1, memory_order_seq_cst);

  __atomic_fetch_and(i, 3, memory_order_seq_cst);
  __atomic_fetch_or(I, 3, memory_order_seq_cst);
  __atomic_fetch_xor(P, 3, memory_order_seq_cst);
  __atomic_fetch_or(F, 3, memory_order_seq_cst);
  __atomic_fetch_and(s1, 3, memory_order_seq_cst);

  _Bool cmpexch_1 = __c11_atomic_compare_exchange_strong(i, I, 1, memory_order_seq_cst, memory_order_seq_cst);
  _Bool cmpexch_2 = __c11_atomic_compare_exchange_strong(p, P, (int*)1, memory_order_seq_cst, memory_order_seq_cst);
  _Bool cmpexch_3 = __c11_atomic_compare_exchange_strong(f, I, 1, memory_order_seq_cst, memory_order_seq_cst);
  (void)__c11_atomic_compare_exchange_strong(i, CI, 1, memory_order_seq_cst, memory_order_seq_cst);

  _Bool cmpexchw_1 = __c11_atomic_compare_exchange_weak(i, I, 1, memory_order_seq_cst, memory_order_seq_cst);
  _Bool cmpexchw_2 = __c11_atomic_compare_exchange_weak(p, P, (int*)1, memory_order_seq_cst, memory_order_seq_cst);
  _Bool cmpexchw_3 = __c11_atomic_compare_exchange_weak(f, I, 1, memory_order_seq_cst, memory_order_seq_cst);
  (void)__c11_atomic_compare_exchange_weak(i, CI, 1, memory_order_seq_cst, memory_order_seq_cst);

  _Bool cmpexch_4 = __atomic_compare_exchange_n(I, I, 5, 1, memory_order_seq_cst, memory_order_seq_cst);
  _Bool cmpexch_5 = __atomic_compare_exchange_n(I, P, 5, 0, memory_order_seq_cst, memory_order_seq_cst);
  _Bool cmpexch_6 = __atomic_compare_exchange_n(I, I, P, 0, memory_order_seq_cst, memory_order_seq_cst);
  (void)__atomic_compare_exchange_n(CI, I, 5, 1, memory_order_seq_cst, memory_order_seq_cst);
  (void)__atomic_compare_exchange_n(I, CI, 5, 1, memory_order_seq_cst, memory_order_seq_cst);

  _Bool cmpexch_7 = __atomic_compare_exchange(I, I, 5, 1, memory_order_seq_cst, memory_order_seq_cst);
  _Bool cmpexch_8 = __atomic_compare_exchange(I, P, I, 0, memory_order_seq_cst, memory_order_seq_cst);
  _Bool cmpexch_9 = __atomic_compare_exchange(I, I, I, 0, memory_order_seq_cst, memory_order_seq_cst);
  (void)__atomic_compare_exchange(CI, I, I, 0, memory_order_seq_cst, memory_order_seq_cst);
  (void)__atomic_compare_exchange(I, CI, I, 0, memory_order_seq_cst, memory_order_seq_cst);


  _Bool cmpexch_10 = __c11_atomic_compare_exchange_strong((_Atomic int __attribute__((address_space(1))) *)0x308, (int __attribute__((address_space(2))) *)0x309, 1, memory_order_seq_cst, memory_order_seq_cst);

  const volatile int flag_k = 0;
  volatile int flag = 0;
  (void)(int)__atomic_test_and_set(&flag_k, memory_order_seq_cst);
  (void)(int)__atomic_test_and_set(&flag, memory_order_seq_cst);
  __atomic_clear(&flag_k, memory_order_seq_cst);
  __atomic_clear(&flag, memory_order_seq_cst);
  (int)__atomic_clear(&flag, memory_order_seq_cst);
  __atomic_clear(0x8000, memory_order_seq_cst);
  __atomic_clear(&flag, memory_order_consume);
  __atomic_clear(&flag, memory_order_acquire);
  __atomic_clear(&flag, memory_order_acq_rel);
  _Bool lock;
  __atomic_test_and_set(lock, memory_order_acquire);
  __atomic_clear(lock, memory_order_release);



  __atomic_test_and_set((void*)0x8000, memory_order_seq_cst);
  __atomic_test_and_set((char*)0x8000, memory_order_seq_cst);
  __atomic_test_and_set((int*)0x8000, memory_order_seq_cst);
  __atomic_test_and_set((struct incomplete*)0x8000, memory_order_seq_cst);
  __atomic_clear((void*)0x8000, memory_order_seq_cst);
  __atomic_clear((char*)0x8000, memory_order_seq_cst);
  __atomic_clear((int*)0x8000, memory_order_seq_cst);
  __atomic_clear((struct incomplete*)0x8000, memory_order_seq_cst);

  __c11_atomic_init(ci, 0);
  __c11_atomic_store(ci, 0, memory_order_release);
  __c11_atomic_load(ci, memory_order_acquire);


  atomic_int n = (123);
  __c11_atomic_init(&n, 456);
  __c11_atomic_init(&n, (void*)0);

  const atomic_wchar_t cawt;
  __c11_atomic_init(&cawt, L'x');
  atomic_wchar_t awt;
  __c11_atomic_init(&awt, L'x');

  int x = (12);

  __c11_atomic_thread_fence();
  __c11_atomic_thread_fence(memory_order_seq_cst);
  __c11_atomic_signal_fence(memory_order_seq_cst);
  void (*pfn)(memory_order) = &atomic_thread_fence;
  pfn = &atomic_signal_fence;

  int k = __c11_atomic_load(&n, memory_order_relaxed);
  __c11_atomic_store(&n, k, memory_order_relaxed);
  __c11_atomic_store(&n, __c11_atomic_load(&n, 5), 5);

  k = __c11_atomic_exchange(&n, 72, 5);
  k = __c11_atomic_exchange(&n, k, memory_order_release);

  __c11_atomic_compare_exchange_strong(&n, k, k, 5, 5);
  __c11_atomic_compare_exchange_weak(&n, &k, k, 5, 5);
  __c11_atomic_compare_exchange_strong(&n, &k, k, memory_order_seq_cst);
  __c11_atomic_compare_exchange_weak(&n, &k, k, memory_order_seq_cst, memory_order_acquire);

  __c11_atomic_fetch_add(&k, n, 5);
  k = __c11_atomic_fetch_add(&n, k, 5);
  k = __c11_atomic_fetch_sub(&n, k, 5);
  k = __c11_atomic_fetch_and(&n, k, 5);
  k = __c11_atomic_fetch_or(&n, k, 5);
  k = __c11_atomic_fetch_xor(&n, k, 5);
  k = __c11_atomic_fetch_add(&n, k, memory_order_acquire);
  k = __c11_atomic_fetch_sub(&n, k, memory_order_release);
  k = __c11_atomic_fetch_and(&n, k, memory_order_acq_rel);
  k = __c11_atomic_fetch_or(&n, k, memory_order_consume);
  k = __c11_atomic_fetch_xor(&n, k, memory_order_relaxed);


  struct atomic_flag must_be_struct = { 0 };

  atomic_flag guard = { 0 };
  _Bool old_val = __c11_atomic_exchange(&(&guard)->_Value, 1, 5);
  if (old_val) __c11_atomic_store(&(&guard)->_Value, 0, 5);

  old_val = (atomic_flag_test_and_set)(&guard);
  if (old_val) (atomic_flag_clear)(&guard);

  const atomic_flag const_guard;
  __c11_atomic_exchange(&(&const_guard)->_Value, 1, 5);
  __c11_atomic_store(&(&const_guard)->_Value, 0, 5);
}

_Atomic(int*) PR12527_a;
void PR12527(void) { int *b = PR12527_a; }

void PR16931(int* x) {
  typedef struct { _Atomic(_Bool) flag; } flag;
  flag flagvar = { 0 };
  PR16931(&flagvar);
}

void memory_checks(_Atomic(int) *Ap, int *p, int val) {
  (void)__c11_atomic_load(Ap, memory_order_relaxed);
  (void)__c11_atomic_load(Ap, memory_order_acquire);
  (void)__c11_atomic_load(Ap, memory_order_consume);
  (void)__c11_atomic_load(Ap, memory_order_release);
  (void)__c11_atomic_load(Ap, memory_order_acq_rel);
  (void)__c11_atomic_load(Ap, memory_order_seq_cst);
  (void)__c11_atomic_load(Ap, val);
  (void)__c11_atomic_load(Ap, -1);
  (void)__c11_atomic_load(Ap, 42);

  (void)__c11_atomic_store(Ap, val, memory_order_relaxed);
  (void)__c11_atomic_store(Ap, val, memory_order_acquire);
  (void)__c11_atomic_store(Ap, val, memory_order_consume);
  (void)__c11_atomic_store(Ap, val, memory_order_release);
  (void)__c11_atomic_store(Ap, val, memory_order_acq_rel);
  (void)__c11_atomic_store(Ap, val, memory_order_seq_cst);

  (void)__c11_atomic_fetch_add(Ap, 1, memory_order_relaxed);
  (void)__c11_atomic_fetch_add(Ap, 1, memory_order_acquire);
  (void)__c11_atomic_fetch_add(Ap, 1, memory_order_consume);
  (void)__c11_atomic_fetch_add(Ap, 1, memory_order_release);
  (void)__c11_atomic_fetch_add(Ap, 1, memory_order_acq_rel);
  (void)__c11_atomic_fetch_add(Ap, 1, memory_order_seq_cst);

  (void)__c11_atomic_fetch_add(
      (struct Incomplete * _Atomic *)0,
      1, memory_order_seq_cst);

  (void)__c11_atomic_init(Ap, val);
  (void)__c11_atomic_init(Ap, val);
  (void)__c11_atomic_init(Ap, val);
  (void)__c11_atomic_init(Ap, val);
  (void)__c11_atomic_init(Ap, val);
  (void)__c11_atomic_init(Ap, val);

  (void)__c11_atomic_fetch_sub(Ap, val, memory_order_relaxed);
  (void)__c11_atomic_fetch_sub(Ap, val, memory_order_acquire);
  (void)__c11_atomic_fetch_sub(Ap, val, memory_order_consume);
  (void)__c11_atomic_fetch_sub(Ap, val, memory_order_release);
  (void)__c11_atomic_fetch_sub(Ap, val, memory_order_acq_rel);
  (void)__c11_atomic_fetch_sub(Ap, val, memory_order_seq_cst);

  (void)__c11_atomic_fetch_and(Ap, val, memory_order_relaxed);
  (void)__c11_atomic_fetch_and(Ap, val, memory_order_acquire);
  (void)__c11_atomic_fetch_and(Ap, val, memory_order_consume);
  (void)__c11_atomic_fetch_and(Ap, val, memory_order_release);
  (void)__c11_atomic_fetch_and(Ap, val, memory_order_acq_rel);
  (void)__c11_atomic_fetch_and(Ap, val, memory_order_seq_cst);

  (void)__c11_atomic_fetch_or(Ap, val, memory_order_relaxed);
  (void)__c11_atomic_fetch_or(Ap, val, memory_order_acquire);
  (void)__c11_atomic_fetch_or(Ap, val, memory_order_consume);
  (void)__c11_atomic_fetch_or(Ap, val, memory_order_release);
  (void)__c11_atomic_fetch_or(Ap, val, memory_order_acq_rel);
  (void)__c11_atomic_fetch_or(Ap, val, memory_order_seq_cst);

  (void)__c11_atomic_fetch_xor(Ap, val, memory_order_relaxed);
  (void)__c11_atomic_fetch_xor(Ap, val, memory_order_acquire);
  (void)__c11_atomic_fetch_xor(Ap, val, memory_order_consume);
  (void)__c11_atomic_fetch_xor(Ap, val, memory_order_release);
  (void)__c11_atomic_fetch_xor(Ap, val, memory_order_acq_rel);
  (void)__c11_atomic_fetch_xor(Ap, val, memory_order_seq_cst);

  (void)__c11_atomic_fetch_nand(Ap, val, memory_order_relaxed);
  (void)__c11_atomic_fetch_nand(Ap, val, memory_order_acquire);
  (void)__c11_atomic_fetch_nand(Ap, val, memory_order_consume);
  (void)__c11_atomic_fetch_nand(Ap, val, memory_order_release);
  (void)__c11_atomic_fetch_nand(Ap, val, memory_order_acq_rel);
  (void)__c11_atomic_fetch_nand(Ap, val, memory_order_seq_cst);

  (void)__c11_atomic_fetch_min(Ap, val, memory_order_relaxed);
  (void)__c11_atomic_fetch_min(Ap, val, memory_order_acquire);
  (void)__c11_atomic_fetch_min(Ap, val, memory_order_consume);
  (void)__c11_atomic_fetch_min(Ap, val, memory_order_release);
  (void)__c11_atomic_fetch_min(Ap, val, memory_order_acq_rel);
  (void)__c11_atomic_fetch_min(Ap, val, memory_order_seq_cst);

  (void)__c11_atomic_fetch_max(Ap, val, memory_order_relaxed);
  (void)__c11_atomic_fetch_max(Ap, val, memory_order_acquire);
  (void)__c11_atomic_fetch_max(Ap, val, memory_order_consume);
  (void)__c11_atomic_fetch_max(Ap, val, memory_order_release);
  (void)__c11_atomic_fetch_max(Ap, val, memory_order_acq_rel);
  (void)__c11_atomic_fetch_max(Ap, val, memory_order_seq_cst);

  (void)__c11_atomic_exchange(Ap, val, memory_order_relaxed);
  (void)__c11_atomic_exchange(Ap, val, memory_order_acquire);
  (void)__c11_atomic_exchange(Ap, val, memory_order_consume);
  (void)__c11_atomic_exchange(Ap, val, memory_order_release);
  (void)__c11_atomic_exchange(Ap, val, memory_order_acq_rel);
  (void)__c11_atomic_exchange(Ap, val, memory_order_seq_cst);

  (void)__c11_atomic_compare_exchange_strong(Ap, p, val, -1, memory_order_relaxed);
  (void)__c11_atomic_compare_exchange_strong(Ap, p, val, memory_order_relaxed, memory_order_relaxed);
  (void)__c11_atomic_compare_exchange_strong(Ap, p, val, memory_order_acquire, memory_order_relaxed);
  (void)__c11_atomic_compare_exchange_strong(Ap, p, val, memory_order_consume, memory_order_relaxed);
  (void)__c11_atomic_compare_exchange_strong(Ap, p, val, memory_order_release, memory_order_relaxed);
  (void)__c11_atomic_compare_exchange_strong(Ap, p, val, memory_order_acq_rel, memory_order_relaxed);
  (void)__c11_atomic_compare_exchange_strong(Ap, p, val, memory_order_seq_cst, memory_order_acquire);
  (void)__c11_atomic_compare_exchange_strong(Ap, p, val, memory_order_seq_cst, memory_order_consume);
  (void)__c11_atomic_compare_exchange_strong(Ap, p, val, memory_order_seq_cst, memory_order_release);
  (void)__c11_atomic_compare_exchange_strong(Ap, p, val, memory_order_seq_cst, memory_order_acq_rel);
  (void)__c11_atomic_compare_exchange_strong(Ap, p, val, memory_order_seq_cst, memory_order_seq_cst);
  (void)__c11_atomic_compare_exchange_strong(Ap, p, val, memory_order_seq_cst, -1);
  (void)__c11_atomic_compare_exchange_strong(Ap, p, val, memory_order_relaxed, memory_order_acquire);
  (void)__c11_atomic_compare_exchange_strong(Ap, p, val, memory_order_acquire, memory_order_seq_cst);

  (void)__c11_atomic_compare_exchange_weak(Ap, p, val, -1, memory_order_relaxed);
  (void)__c11_atomic_compare_exchange_weak(Ap, p, val, memory_order_relaxed, memory_order_relaxed);
  (void)__c11_atomic_compare_exchange_weak(Ap, p, val, memory_order_acquire, memory_order_relaxed);
  (void)__c11_atomic_compare_exchange_weak(Ap, p, val, memory_order_consume, memory_order_relaxed);
  (void)__c11_atomic_compare_exchange_weak(Ap, p, val, memory_order_release, memory_order_relaxed);
  (void)__c11_atomic_compare_exchange_weak(Ap, p, val, memory_order_acq_rel, memory_order_relaxed);
  (void)__c11_atomic_compare_exchange_weak(Ap, p, val, memory_order_seq_cst, memory_order_acquire);
  (void)__c11_atomic_compare_exchange_weak(Ap, p, val, memory_order_seq_cst, memory_order_consume);
  (void)__c11_atomic_compare_exchange_weak(Ap, p, val, memory_order_seq_cst, memory_order_release);
  (void)__c11_atomic_compare_exchange_weak(Ap, p, val, memory_order_seq_cst, memory_order_acq_rel);
  (void)__c11_atomic_compare_exchange_weak(Ap, p, val, memory_order_seq_cst, memory_order_seq_cst);
  (void)__c11_atomic_compare_exchange_weak(Ap, p, val, memory_order_seq_cst, -1);
  (void)__c11_atomic_compare_exchange_weak(Ap, p, val, memory_order_relaxed, memory_order_acquire);
  (void)__c11_atomic_compare_exchange_weak(Ap, p, val, memory_order_acquire, memory_order_seq_cst);

  (void)__atomic_load_n(p, memory_order_relaxed);
  (void)__atomic_load_n(p, memory_order_acquire);
  (void)__atomic_load_n(p, memory_order_consume);
  (void)__atomic_load_n(p, memory_order_release);
  (void)__atomic_load_n(p, memory_order_acq_rel);
  (void)__atomic_load_n(p, memory_order_seq_cst);

  (void)__atomic_load(p, p, memory_order_relaxed);
  (void)__atomic_load(p, p, memory_order_acquire);
  (void)__atomic_load(p, p, memory_order_consume);
  (void)__atomic_load(p, p, memory_order_release);
  (void)__atomic_load(p, p, memory_order_acq_rel);
  (void)__atomic_load(p, p, memory_order_seq_cst);

  (void)__atomic_store(p, p, memory_order_relaxed);
  (void)__atomic_store(p, p, memory_order_acquire);
  (void)__atomic_store(p, p, memory_order_consume);
  (void)__atomic_store(p, p, memory_order_release);
  (void)__atomic_store(p, p, memory_order_acq_rel);
  (void)__atomic_store(p, p, memory_order_seq_cst);

  (void)__atomic_store_n(p, val, memory_order_relaxed);
  (void)__atomic_store_n(p, val, memory_order_acquire);
  (void)__atomic_store_n(p, val, memory_order_consume);
  (void)__atomic_store_n(p, val, memory_order_release);
  (void)__atomic_store_n(p, val, memory_order_acq_rel);
  (void)__atomic_store_n(p, val, memory_order_seq_cst);

  (void)__atomic_fetch_add(p, val, memory_order_relaxed);
  (void)__atomic_fetch_add(p, val, memory_order_acquire);
  (void)__atomic_fetch_add(p, val, memory_order_consume);
  (void)__atomic_fetch_add(p, val, memory_order_release);
  (void)__atomic_fetch_add(p, val, memory_order_acq_rel);
  (void)__atomic_fetch_add(p, val, memory_order_seq_cst);

  (void)__atomic_fetch_sub(p, val, memory_order_relaxed);
  (void)__atomic_fetch_sub(p, val, memory_order_acquire);
  (void)__atomic_fetch_sub(p, val, memory_order_consume);
  (void)__atomic_fetch_sub(p, val, memory_order_release);
  (void)__atomic_fetch_sub(p, val, memory_order_acq_rel);
  (void)__atomic_fetch_sub(p, val, memory_order_seq_cst);

  (void)__atomic_add_fetch(p, val, memory_order_relaxed);
  (void)__atomic_add_fetch(p, val, memory_order_acquire);
  (void)__atomic_add_fetch(p, val, memory_order_consume);
  (void)__atomic_add_fetch(p, val, memory_order_release);
  (void)__atomic_add_fetch(p, val, memory_order_acq_rel);
  (void)__atomic_add_fetch(p, val, memory_order_seq_cst);

  (void)__atomic_sub_fetch(p, val, memory_order_relaxed);
  (void)__atomic_sub_fetch(p, val, memory_order_acquire);
  (void)__atomic_sub_fetch(p, val, memory_order_consume);
  (void)__atomic_sub_fetch(p, val, memory_order_release);
  (void)__atomic_sub_fetch(p, val, memory_order_acq_rel);
  (void)__atomic_sub_fetch(p, val, memory_order_seq_cst);

  (void)__atomic_fetch_and(p, val, memory_order_relaxed);
  (void)__atomic_fetch_and(p, val, memory_order_acquire);
  (void)__atomic_fetch_and(p, val, memory_order_consume);
  (void)__atomic_fetch_and(p, val, memory_order_release);
  (void)__atomic_fetch_and(p, val, memory_order_acq_rel);
  (void)__atomic_fetch_and(p, val, memory_order_seq_cst);

  (void)__atomic_fetch_or(p, val, memory_order_relaxed);
  (void)__atomic_fetch_or(p, val, memory_order_acquire);
  (void)__atomic_fetch_or(p, val, memory_order_consume);
  (void)__atomic_fetch_or(p, val, memory_order_release);
  (void)__atomic_fetch_or(p, val, memory_order_acq_rel);
  (void)__atomic_fetch_or(p, val, memory_order_seq_cst);

  (void)__atomic_fetch_xor(p, val, memory_order_relaxed);
  (void)__atomic_fetch_xor(p, val, memory_order_acquire);
  (void)__atomic_fetch_xor(p, val, memory_order_consume);
  (void)__atomic_fetch_xor(p, val, memory_order_release);
  (void)__atomic_fetch_xor(p, val, memory_order_acq_rel);
  (void)__atomic_fetch_xor(p, val, memory_order_seq_cst);

  (void)__atomic_fetch_nand(p, val, memory_order_relaxed);
  (void)__atomic_fetch_nand(p, val, memory_order_acquire);
  (void)__atomic_fetch_nand(p, val, memory_order_consume);
  (void)__atomic_fetch_nand(p, val, memory_order_release);
  (void)__atomic_fetch_nand(p, val, memory_order_acq_rel);
  (void)__atomic_fetch_nand(p, val, memory_order_seq_cst);

  (void)__atomic_fetch_min(p, val, memory_order_relaxed);
  (void)__atomic_fetch_min(p, val, memory_order_acquire);
  (void)__atomic_fetch_min(p, val, memory_order_consume);
  (void)__atomic_fetch_min(p, val, memory_order_release);
  (void)__atomic_fetch_min(p, val, memory_order_acq_rel);
  (void)__atomic_fetch_min(p, val, memory_order_seq_cst);

  (void)__atomic_fetch_uinc(p, val, memory_order_relaxed);
  (void)__atomic_fetch_uinc(p, val, memory_order_acquire);
  (void)__atomic_fetch_uinc(p, val, memory_order_consume);
  (void)__atomic_fetch_uinc(p, val, memory_order_release);
  (void)__atomic_fetch_uinc(p, val, memory_order_acq_rel);
  (void)__atomic_fetch_uinc(p, val, memory_order_seq_cst);

  (void)__atomic_fetch_udec(p, val, memory_order_relaxed);
  (void)__atomic_fetch_udec(p, val, memory_order_acquire);
  (void)__atomic_fetch_udec(p, val, memory_order_consume);
  (void)__atomic_fetch_udec(p, val, memory_order_release);
  (void)__atomic_fetch_udec(p, val, memory_order_acq_rel);
  (void)__atomic_fetch_udec(p, val, memory_order_seq_cst);

  (void)__atomic_fetch_max(p, val, memory_order_relaxed);
  (void)__atomic_fetch_max(p, val, memory_order_acquire);
  (void)__atomic_fetch_max(p, val, memory_order_consume);
  (void)__atomic_fetch_max(p, val, memory_order_release);
  (void)__atomic_fetch_max(p, val, memory_order_acq_rel);
  (void)__atomic_fetch_max(p, val, memory_order_seq_cst);

  (void)__atomic_and_fetch(p, val, memory_order_relaxed);
  (void)__atomic_and_fetch(p, val, memory_order_acquire);
  (void)__atomic_and_fetch(p, val, memory_order_consume);
  (void)__atomic_and_fetch(p, val, memory_order_release);
  (void)__atomic_and_fetch(p, val, memory_order_acq_rel);
  (void)__atomic_and_fetch(p, val, memory_order_seq_cst);

  (void)__atomic_or_fetch(p, val, memory_order_relaxed);
  (void)__atomic_or_fetch(p, val, memory_order_acquire);
  (void)__atomic_or_fetch(p, val, memory_order_consume);
  (void)__atomic_or_fetch(p, val, memory_order_release);
  (void)__atomic_or_fetch(p, val, memory_order_acq_rel);
  (void)__atomic_or_fetch(p, val, memory_order_seq_cst);

  (void)__atomic_xor_fetch(p, val, memory_order_relaxed);
  (void)__atomic_xor_fetch(p, val, memory_order_acquire);
  (void)__atomic_xor_fetch(p, val, memory_order_consume);
  (void)__atomic_xor_fetch(p, val, memory_order_release);
  (void)__atomic_xor_fetch(p, val, memory_order_acq_rel);
  (void)__atomic_xor_fetch(p, val, memory_order_seq_cst);

  (void)__atomic_nand_fetch(p, val, memory_order_relaxed);
  (void)__atomic_nand_fetch(p, val, memory_order_acquire);
  (void)__atomic_nand_fetch(p, val, memory_order_consume);
  (void)__atomic_nand_fetch(p, val, memory_order_release);
  (void)__atomic_nand_fetch(p, val, memory_order_acq_rel);
  (void)__atomic_nand_fetch(p, val, memory_order_seq_cst);

  (void)__atomic_max_fetch(p, val, memory_order_relaxed);
  (void)__atomic_max_fetch(p, val, memory_order_acquire);
  (void)__atomic_max_fetch(p, val, memory_order_consume);
  (void)__atomic_max_fetch(p, val, memory_order_release);
  (void)__atomic_max_fetch(p, val, memory_order_acq_rel);
  (void)__atomic_max_fetch(p, val, memory_order_seq_cst);

  (void)__atomic_min_fetch(p, val, memory_order_relaxed);
  (void)__atomic_min_fetch(p, val, memory_order_acquire);
  (void)__atomic_min_fetch(p, val, memory_order_consume);
  (void)__atomic_min_fetch(p, val, memory_order_release);
  (void)__atomic_min_fetch(p, val, memory_order_acq_rel);
  (void)__atomic_min_fetch(p, val, memory_order_seq_cst);

  (void)__atomic_exchange_n(p, val, memory_order_relaxed);
  (void)__atomic_exchange_n(p, val, memory_order_acquire);
  (void)__atomic_exchange_n(p, val, memory_order_consume);
  (void)__atomic_exchange_n(p, val, memory_order_release);
  (void)__atomic_exchange_n(p, val, memory_order_acq_rel);
  (void)__atomic_exchange_n(p, val, memory_order_seq_cst);

  (void)__atomic_exchange(p, p, p, memory_order_relaxed);
  (void)__atomic_exchange(p, p, p, memory_order_acquire);
  (void)__atomic_exchange(p, p, p, memory_order_consume);
  (void)__atomic_exchange(p, p, p, memory_order_release);
  (void)__atomic_exchange(p, p, p, memory_order_acq_rel);
  (void)__atomic_exchange(p, p, p, memory_order_seq_cst);

  (void)__atomic_compare_exchange(p, p, p, 0, -1, memory_order_relaxed);
  (void)__atomic_compare_exchange(p, p, p, 0, memory_order_relaxed, memory_order_relaxed);
  (void)__atomic_compare_exchange(p, p, p, 0, memory_order_acquire, memory_order_relaxed);
  (void)__atomic_compare_exchange(p, p, p, 0, memory_order_consume, memory_order_relaxed);
  (void)__atomic_compare_exchange(p, p, p, 0, memory_order_release, memory_order_relaxed);
  (void)__atomic_compare_exchange(p, p, p, 0, memory_order_acq_rel, memory_order_relaxed);
  (void)__atomic_compare_exchange(p, p, p, 0, memory_order_seq_cst, memory_order_acquire);
  (void)__atomic_compare_exchange(p, p, p, 0, memory_order_seq_cst, memory_order_consume);
  (void)__atomic_compare_exchange(p, p, p, 0, memory_order_seq_cst, memory_order_release);
  (void)__atomic_compare_exchange(p, p, p, 0, memory_order_seq_cst, memory_order_acq_rel);
  (void)__atomic_compare_exchange(p, p, p, 0, memory_order_seq_cst, memory_order_seq_cst);
  (void)__atomic_compare_exchange(p, p, p, 0, memory_order_seq_cst, -1);

  (void)__atomic_compare_exchange_n(p, p, val, 0, -1, memory_order_relaxed);
  (void)__atomic_compare_exchange_n(p, p, val, 0, memory_order_relaxed, memory_order_relaxed);
  (void)__atomic_compare_exchange_n(p, p, val, 0, memory_order_acquire, memory_order_relaxed);
  (void)__atomic_compare_exchange_n(p, p, val, 0, memory_order_consume, memory_order_relaxed);
  (void)__atomic_compare_exchange_n(p, p, val, 0, memory_order_release, memory_order_relaxed);
  (void)__atomic_compare_exchange_n(p, p, val, 0, memory_order_acq_rel, memory_order_relaxed);
  (void)__atomic_compare_exchange_n(p, p, val, 0, memory_order_seq_cst, memory_order_relaxed);
  (void)__atomic_compare_exchange_n(p, p, val, 0, memory_order_seq_cst, memory_order_acquire);
  (void)__atomic_compare_exchange_n(p, p, val, 0, memory_order_seq_cst, memory_order_consume);
  (void)__atomic_compare_exchange_n(p, p, val, 0, memory_order_seq_cst, memory_order_release);
  (void)__atomic_compare_exchange_n(p, p, val, 0, memory_order_seq_cst, memory_order_acq_rel);
  (void)__atomic_compare_exchange_n(p, p, val, 0, memory_order_seq_cst, memory_order_seq_cst);
  (void)__atomic_compare_exchange_n(p, p, val, 0, memory_order_seq_cst, -1);
}

struct Z {
  char z[];
};

void zeroSizeArgError(struct Z *a, struct Z *b, struct Z *c) {
  __atomic_exchange(b, b, c, memory_order_relaxed);
  __atomic_exchange(b, b, c, memory_order_acq_rel);
  __atomic_exchange(b, b, c, memory_order_acquire);
  __atomic_exchange(b, b, c, memory_order_consume);
  __atomic_exchange(b, b, c, memory_order_release);
  __atomic_exchange(b, b, c, memory_order_seq_cst);
  __atomic_load(a, b, memory_order_relaxed);
  __atomic_load(a, b, memory_order_acq_rel);
  __atomic_load(a, b, memory_order_acquire);
  __atomic_load(a, b, memory_order_consume);
  __atomic_load(a, b, memory_order_release);
  __atomic_load(a, b, memory_order_seq_cst);
  __atomic_store(a, b, memory_order_relaxed);
  __atomic_store(a, b, memory_order_acq_rel);
  __atomic_store(a, b, memory_order_acquire);
  __atomic_store(a, b, memory_order_consume);
  __atomic_store(a, b, memory_order_release);
  __atomic_store(a, b, memory_order_seq_cst);
  __atomic_compare_exchange(a, b, c, 0, memory_order_relaxed, memory_order_relaxed);
  __atomic_compare_exchange(a, b, c, 0, memory_order_acq_rel, memory_order_acq_rel);
  __atomic_compare_exchange(a, b, c, 0, memory_order_acquire, memory_order_acquire);
  __atomic_compare_exchange(a, b, c, 0, memory_order_consume, memory_order_consume);
  __atomic_compare_exchange(a, b, c, 0, memory_order_release, memory_order_release);
  __atomic_compare_exchange(a, b, c, 0, memory_order_seq_cst, memory_order_seq_cst);

}

struct IncompleteTy IncA, IncB, IncC;

void incompleteTypeArgError() {
  __atomic_exchange(&IncB, &IncB, &IncC, memory_order_relaxed);
  __atomic_exchange(&IncB, &IncB, &IncC, memory_order_acq_rel);
  __atomic_exchange(&IncB, &IncB, &IncC, memory_order_acquire);
  __atomic_exchange(&IncB, &IncB, &IncC, memory_order_consume);
  __atomic_exchange(&IncB, &IncB, &IncC, memory_order_release);
  __atomic_exchange(&IncB, &IncB, &IncC, memory_order_seq_cst);
  __atomic_load(&IncA, &IncB, memory_order_relaxed);
  __atomic_load(&IncA, &IncB, memory_order_acq_rel);
  __atomic_load(&IncA, &IncB, memory_order_acquire);
  __atomic_load(&IncA, &IncB, memory_order_consume);
  __atomic_load(&IncA, &IncB, memory_order_release);
  __atomic_load(&IncA, &IncB, memory_order_seq_cst);
  __atomic_store(&IncA, &IncB, memory_order_relaxed);
  __atomic_store(&IncA, &IncB, memory_order_acq_rel);
  __atomic_store(&IncA, &IncB, memory_order_acquire);
  __atomic_store(&IncA, &IncB, memory_order_consume);
  __atomic_store(&IncA, &IncB, memory_order_release);
  __atomic_store(&IncA, &IncB, memory_order_seq_cst);
  __atomic_compare_exchange(&IncA, &IncB, &IncC, 0, memory_order_relaxed, memory_order_relaxed);
  __atomic_compare_exchange(&IncA, &IncB, &IncC, 0, memory_order_acq_rel, memory_order_acq_rel);
  __atomic_compare_exchange(&IncA, &IncB, &IncC, 0, memory_order_acquire, memory_order_acquire);
  __atomic_compare_exchange(&IncA, &IncB, &IncC, 0, memory_order_consume, memory_order_consume);
  __atomic_compare_exchange(&IncA, &IncB, &IncC, 0, memory_order_release, memory_order_release);
  __atomic_compare_exchange(&IncA, &IncB, &IncC, 0, memory_order_seq_cst, memory_order_seq_cst);

}

void nullPointerWarning(void) {
  volatile _Atomic(int) vai;
  _Atomic(int) ai;
  volatile int vi = 42;
  int i = 42;
  volatile _Atomic(int*) vap;
  _Atomic(int*) ap;
  volatile int* vp = ((void*)0);
  int* p = ((void*)0);

  __c11_atomic_init((volatile _Atomic(int)*)0, 42);
  __c11_atomic_init((_Atomic(int)*)0, 42);
  __c11_atomic_store((volatile _Atomic(int)*)0, 42, memory_order_relaxed);
  __c11_atomic_store((_Atomic(int)*)0, 42, memory_order_relaxed);
  (void)__c11_atomic_load((volatile _Atomic(int)*)0, memory_order_relaxed);
  (void)__c11_atomic_load((_Atomic(int)*)0, memory_order_relaxed);
  (void)__c11_atomic_exchange((volatile _Atomic(int)*)0, 42, memory_order_relaxed);
  (void)__c11_atomic_exchange((_Atomic(int)*)0, 42, memory_order_relaxed);
  (void)__c11_atomic_compare_exchange_weak((volatile _Atomic(int)*)0, &i, 42, memory_order_relaxed, memory_order_relaxed);
  (void)__c11_atomic_compare_exchange_weak((_Atomic(int)*)0, &i, 42, memory_order_relaxed, memory_order_relaxed);
  (void)__c11_atomic_compare_exchange_weak(&vai, (int*)0, 42, memory_order_relaxed, memory_order_relaxed);
  (void)__c11_atomic_compare_exchange_weak(&ai, (int*)0, 42, memory_order_relaxed, memory_order_relaxed);
  (void)__c11_atomic_compare_exchange_strong((volatile _Atomic(int)*)0, &i, 42, memory_order_relaxed, memory_order_relaxed);
  (void)__c11_atomic_compare_exchange_strong((_Atomic(int)*)0, &i, 42, memory_order_relaxed, memory_order_relaxed);
  (void)__c11_atomic_compare_exchange_strong(&vai, (int*)0, 42, memory_order_relaxed, memory_order_relaxed);
  (void)__c11_atomic_compare_exchange_strong(&ai, (int*)0, 42, memory_order_relaxed, memory_order_relaxed);
  (void)__c11_atomic_fetch_add((volatile _Atomic(int)*)0, 42, memory_order_relaxed);
  (void)__c11_atomic_fetch_add((_Atomic(int)*)0, 42, memory_order_relaxed);
  (void)__c11_atomic_fetch_sub((volatile _Atomic(int)*)0, 42, memory_order_relaxed);
  (void)__c11_atomic_fetch_sub((_Atomic(int)*)0, 42, memory_order_relaxed);
  (void)__c11_atomic_fetch_and((volatile _Atomic(int)*)0, 42, memory_order_relaxed);
  (void)__c11_atomic_fetch_and((_Atomic(int)*)0, 42, memory_order_relaxed);
  (void)__c11_atomic_fetch_or((volatile _Atomic(int)*)0, 42, memory_order_relaxed);
  (void)__c11_atomic_fetch_or((_Atomic(int)*)0, 42, memory_order_relaxed);
  (void)__c11_atomic_fetch_xor((volatile _Atomic(int)*)0, 42, memory_order_relaxed);
  (void)__c11_atomic_fetch_xor((_Atomic(int)*)0, 42, memory_order_relaxed);
  (void)__c11_atomic_fetch_nand((volatile _Atomic(int)*)0, 42, memory_order_relaxed);
  (void)__c11_atomic_fetch_nand((_Atomic(int)*)0, 42, memory_order_relaxed);

  __atomic_store_n((volatile int*)0, 42, memory_order_relaxed);
  __atomic_store_n((int*)0, 42, memory_order_relaxed);
  __atomic_store((volatile int*)0, &i, memory_order_relaxed);
  __atomic_store((int*)0, &i, memory_order_relaxed);
  __atomic_store(&vi, (int*)0, memory_order_relaxed);
  __atomic_store(&i, (int*)0, memory_order_relaxed);
  (void)__atomic_load_n((volatile int*)0, memory_order_relaxed);
  (void)__atomic_load_n((int*)0, memory_order_relaxed);
  __atomic_load((volatile int*)0, &i, memory_order_relaxed);
  __atomic_load((int*)0, &i, memory_order_relaxed);
  __atomic_load(&vi, (int*)0, memory_order_relaxed);
  __atomic_load(&i, (int*)0, memory_order_relaxed);
  (void)__atomic_exchange_n((volatile int*)0, 42, memory_order_relaxed);
  (void)__atomic_exchange_n((int*)0, 42, memory_order_relaxed);
  __atomic_exchange((volatile int*)0, &i, &i, memory_order_relaxed);
  __atomic_exchange((int*)0, &i, &i, memory_order_relaxed);
  __atomic_exchange(&vi, (int*)0, &i, memory_order_relaxed);
  __atomic_exchange(&i, (int*)0, &i, memory_order_relaxed);
  __atomic_exchange(&vi, &i, (int*)0, memory_order_relaxed);
  __atomic_exchange(&i, &i, (int*)0, memory_order_relaxed);
  (void)__atomic_compare_exchange_n((volatile int*)0, &i, 42, 0, memory_order_relaxed, memory_order_relaxed);
  (void)__atomic_compare_exchange_n((int*)0, &i, 42, 0, memory_order_relaxed, memory_order_relaxed);
  (void)__atomic_compare_exchange_n(&vi, (int*)0, 42, 0, memory_order_relaxed, memory_order_relaxed);
  (void)__atomic_compare_exchange_n(&i, (int*)0, 42, 0, memory_order_relaxed, memory_order_relaxed);
  (void)__atomic_compare_exchange((volatile int*)0, &i, &i, 0, memory_order_relaxed, memory_order_relaxed);
  (void)__atomic_compare_exchange((int*)0, &i, &i, 0, memory_order_relaxed, memory_order_relaxed);
  (void)__atomic_compare_exchange(&vi, (int*)0, &i, 0, memory_order_relaxed, memory_order_relaxed);
  (void)__atomic_compare_exchange(&i, (int*)0, &i, 0, memory_order_relaxed, memory_order_relaxed);
  (void)__atomic_compare_exchange(&vi, &i, (int*)0, 0, memory_order_relaxed, memory_order_relaxed);
  (void)__atomic_compare_exchange(&i, &i, (int*)0, 0, memory_order_relaxed, memory_order_relaxed);
  (void)__atomic_fetch_add((volatile int*)0, 42, memory_order_relaxed);
  (void)__atomic_fetch_add((int*)0, 42, memory_order_relaxed);
  (void)__atomic_fetch_sub((volatile int*)0, 42, memory_order_relaxed);
  (void)__atomic_fetch_sub((int*)0, 42, memory_order_relaxed);
  (void)__atomic_fetch_and((volatile int*)0, 42, memory_order_relaxed);
  (void)__atomic_fetch_and((int*)0, 42, memory_order_relaxed);
  (void)__atomic_fetch_or((volatile int*)0, 42, memory_order_relaxed);
  (void)__atomic_fetch_or((int*)0, 42, memory_order_relaxed);
  (void)__atomic_fetch_xor((volatile int*)0, 42, memory_order_relaxed);
  (void)__atomic_fetch_xor((int*)0, 42, memory_order_relaxed);
  (void)__atomic_fetch_min((volatile int*)0, 42, memory_order_relaxed);
  (void)__atomic_fetch_min((int*)0, 42, memory_order_relaxed);
  (void)__atomic_fetch_max((volatile int*)0, 42, memory_order_relaxed);
  (void)__atomic_fetch_max((int*)0, 42, memory_order_relaxed);



  __c11_atomic_init(&vai, 0);
  __c11_atomic_init(&ai, 0);
  __c11_atomic_init(&vap, ((void*)0));
  __c11_atomic_init(&ap, ((void*)0));
  __c11_atomic_store(&vai, 0, memory_order_relaxed);
  __c11_atomic_store(&ai, 0, memory_order_relaxed);
  __c11_atomic_store(&vap, ((void*)0), memory_order_relaxed);
  __c11_atomic_store(&ap, ((void*)0), memory_order_relaxed);
  (void)__c11_atomic_exchange(&vai, 0, memory_order_relaxed);
  (void)__c11_atomic_exchange(&ai, 0, memory_order_relaxed);
  (void)__c11_atomic_exchange(&vap, ((void*)0), memory_order_relaxed);
  (void)__c11_atomic_exchange(&ap, ((void*)0), memory_order_relaxed);
  (void)__c11_atomic_compare_exchange_weak(&vai, &i, 0, memory_order_relaxed, memory_order_relaxed);
  (void)__c11_atomic_compare_exchange_weak(&ai, &i, 0, memory_order_relaxed, memory_order_relaxed);
  (void)__c11_atomic_compare_exchange_weak(&vap, &p, ((void*)0), memory_order_relaxed, memory_order_relaxed);
  (void)__c11_atomic_compare_exchange_weak(&ap, &p, ((void*)0), memory_order_relaxed, memory_order_relaxed);
  (void)__c11_atomic_compare_exchange_strong(&vai, &i, 0, memory_order_relaxed, memory_order_relaxed);
  (void)__c11_atomic_compare_exchange_strong(&ai, &i, 0, memory_order_relaxed, memory_order_relaxed);
  (void)__c11_atomic_compare_exchange_strong(&vap, &p, ((void*)0), memory_order_relaxed, memory_order_relaxed);
  (void)__c11_atomic_compare_exchange_strong(&ap, &p, ((void*)0), memory_order_relaxed, memory_order_relaxed);
  (void)__c11_atomic_fetch_add(&vai, 0, memory_order_relaxed);
  (void)__c11_atomic_fetch_add(&ai, 0, memory_order_relaxed);
  (void)__c11_atomic_fetch_sub(&vai, 0, memory_order_relaxed);
  (void)__c11_atomic_fetch_sub(&ai, 0, memory_order_relaxed);
  (void)__c11_atomic_fetch_and(&vai, 0, memory_order_relaxed);
  (void)__c11_atomic_fetch_and(&ai, 0, memory_order_relaxed);
  (void)__c11_atomic_fetch_or(&vai, 0, memory_order_relaxed);
  (void)__c11_atomic_fetch_or(&ai, 0, memory_order_relaxed);
  (void)__c11_atomic_fetch_xor(&vai, 0, memory_order_relaxed);
  (void)__c11_atomic_fetch_xor(&ai, 0, memory_order_relaxed);
  (void)__c11_atomic_fetch_nand(&vai, 0, memory_order_relaxed);
  (void)__c11_atomic_fetch_nand(&ai, 0, memory_order_relaxed);


  __atomic_store_n(&vi, 0, memory_order_relaxed);
  __atomic_store_n(&i, 0, memory_order_relaxed);
  __atomic_store_n(&vp, ((void*)0), memory_order_relaxed);
  __atomic_store_n(&p, ((void*)0), memory_order_relaxed);
  (void)__atomic_exchange_n(&vi, 0, memory_order_relaxed);
  (void)__atomic_exchange_n(&i, 0, memory_order_relaxed);
  (void)__atomic_exchange_n(&vp, ((void*)0), memory_order_relaxed);
  (void)__atomic_exchange_n(&p, ((void*)0), memory_order_relaxed);
  (void)__atomic_compare_exchange_n(&vi, &i, 0, 0, memory_order_relaxed, memory_order_relaxed);
  (void)__atomic_compare_exchange_n(&i, &i, 0, 0, memory_order_relaxed, memory_order_relaxed);
  (void)__atomic_compare_exchange_n(&vp, &vp, ((void*)0), 0, memory_order_relaxed, memory_order_relaxed);
  (void)__atomic_compare_exchange_n(&p, &p, ((void*)0), 0, memory_order_relaxed, memory_order_relaxed);
  (void)__atomic_fetch_add(&vi, 0, memory_order_relaxed);
  (void)__atomic_fetch_add(&i, 0, memory_order_relaxed);
  (void)__atomic_fetch_sub(&vi, 0, memory_order_relaxed);
  (void)__atomic_fetch_sub(&i, 0, memory_order_relaxed);
  (void)__atomic_fetch_and(&vi, 0, memory_order_relaxed);
  (void)__atomic_fetch_and(&i, 0, memory_order_relaxed);
  (void)__atomic_fetch_or(&vi, 0, memory_order_relaxed);
  (void)__atomic_fetch_or(&i, 0, memory_order_relaxed);
  (void)__atomic_fetch_xor(&vi, 0, memory_order_relaxed);
  (void)__atomic_fetch_xor(&i, 0, memory_order_relaxed);
  (void)__atomic_fetch_min(&vi, 0, memory_order_relaxed);
  (void)__atomic_fetch_min(&i, 0, memory_order_relaxed);
  (void)__atomic_fetch_max(&vi, 0, memory_order_relaxed);
  (void)__atomic_fetch_max(&i, 0, memory_order_relaxed);
}
