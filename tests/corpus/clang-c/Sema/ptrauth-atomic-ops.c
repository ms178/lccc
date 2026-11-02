// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c --target linux_aarch64
# 1 "Sema/ptrauth-atomic-ops.c"
# 1 "<built-in>" 1
# 1 "<built-in>" 3
# 412 "<built-in>" 3
# 1 "<command line>" 1
# 1 "<built-in>" 2
# 1 "Sema/ptrauth-atomic-ops.c" 2



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
# 5 "Sema/ptrauth-atomic-ops.c" 2

int i;
int *__ptrauth(2, 1, 100) authenticated_ptr = &i;
int *__ptrauth(2, 0, 200) non_addr_discriminatedauthenticated_ptr = &i;
int * wat = &i;


void f() {
  static int j = 1;
  __c11_atomic_init((__typeof__(authenticated_ptr) volatile _Atomic *)(long)(&authenticated_ptr), 5);

  __c11_atomic_store((__typeof__(authenticated_ptr) volatile _Atomic *)(long)(&authenticated_ptr), 0, memory_order_relaxed);

  __c11_atomic_load((__typeof__(authenticated_ptr) volatile _Atomic *)(long)(&authenticated_ptr), memory_order_seq_cst);

  __c11_atomic_store((__typeof__(authenticated_ptr) volatile _Atomic *)(long)(&authenticated_ptr), 1, memory_order_seq_cst);

  __atomic_store_n((__typeof__(authenticated_ptr) volatile _Atomic *)(long)(&authenticated_ptr), 4, memory_order_release);

  __atomic_store((__typeof__(authenticated_ptr) volatile _Atomic *)(long)(&authenticated_ptr), j, memory_order_release);

  __c11_atomic_exchange((__typeof__(authenticated_ptr) volatile _Atomic *)(long)(&authenticated_ptr), 1, memory_order_seq_cst);

  __atomic_exchange((__typeof__(authenticated_ptr) volatile _Atomic *)(long)(&authenticated_ptr), &j, &j, memory_order_seq_cst);

  __c11_atomic_fetch_add((__typeof__(authenticated_ptr) volatile _Atomic *)(long)(&authenticated_ptr), 1, memory_order_seq_cst);

  __atomic_fetch_add((__typeof__(authenticated_ptr) volatile _Atomic *)(long)(&authenticated_ptr), 3, memory_order_seq_cst);

  __atomic_fetch_sub((__typeof__(authenticated_ptr) volatile _Atomic *)(long)(&authenticated_ptr), 3, memory_order_seq_cst);

  __atomic_fetch_min((__typeof__(authenticated_ptr) volatile _Atomic *)(long)(&authenticated_ptr), 3, memory_order_seq_cst);

  __atomic_fetch_max((__typeof__(authenticated_ptr) volatile _Atomic *)(long)(&authenticated_ptr), 3, memory_order_seq_cst);

  __c11_atomic_fetch_and((__typeof__(authenticated_ptr) volatile _Atomic *)(long)(&authenticated_ptr), 1, memory_order_seq_cst);

  __atomic_fetch_and((__typeof__(authenticated_ptr) volatile _Atomic *)(long)(&authenticated_ptr), 3, memory_order_seq_cst);

  __atomic_fetch_or((__typeof__(authenticated_ptr) volatile _Atomic *)(long)(&authenticated_ptr), 3, memory_order_seq_cst);

  __atomic_fetch_xor((__typeof__(authenticated_ptr) volatile _Atomic *)(long)(&authenticated_ptr), 3, memory_order_seq_cst);


  __c11_atomic_init((__typeof__(non_addr_discriminatedauthenticated_ptr) volatile _Atomic *)(long)(&non_addr_discriminatedauthenticated_ptr), &j);
  __c11_atomic_store((__typeof__(non_addr_discriminatedauthenticated_ptr) volatile _Atomic *)(long)(&non_addr_discriminatedauthenticated_ptr), 0, memory_order_relaxed);
  __c11_atomic_load((__typeof__(non_addr_discriminatedauthenticated_ptr) volatile _Atomic *)(long)(&non_addr_discriminatedauthenticated_ptr), memory_order_seq_cst);
  __atomic_store(&j, (__typeof__(non_addr_discriminatedauthenticated_ptr) volatile _Atomic *)(long)(&non_addr_discriminatedauthenticated_ptr), memory_order_release);

  __c11_atomic_exchange((__typeof__(j) volatile _Atomic *)(long)(&j), (__typeof__(non_addr_discriminatedauthenticated_ptr) volatile _Atomic *)(long)(&non_addr_discriminatedauthenticated_ptr), memory_order_seq_cst);

  __c11_atomic_fetch_add((__typeof__(non_addr_discriminatedauthenticated_ptr) volatile _Atomic *)(long)(&non_addr_discriminatedauthenticated_ptr), (__typeof__(j) volatile _Atomic *)(long)(&j), memory_order_seq_cst);

  __c11_atomic_fetch_and((__typeof__(j) volatile _Atomic *)(long)(&j), (__typeof__(non_addr_discriminatedauthenticated_ptr) volatile _Atomic *)(long)(&non_addr_discriminatedauthenticated_ptr), memory_order_seq_cst);



  __sync_fetch_and_add(&authenticated_ptr, 1);

  __sync_fetch_and_sub(&authenticated_ptr, 1);

  __sync_fetch_and_or(&authenticated_ptr, 1);

  __sync_fetch_and_and(&authenticated_ptr, 1);

  __sync_fetch_and_xor(&authenticated_ptr, 1);

  __sync_fetch_and_nand(&authenticated_ptr, 1);


  __sync_add_and_fetch(&authenticated_ptr, 1);

  __sync_sub_and_fetch(&authenticated_ptr, 1);

  __sync_or_and_fetch(&authenticated_ptr, 1);

  __sync_and_and_fetch(&authenticated_ptr, 1);

  __sync_xor_and_fetch(&authenticated_ptr, 1);

  __sync_nand_and_fetch(&authenticated_ptr, 1);


  __sync_bool_compare_and_swap(&authenticated_ptr, 1, 0);

  __sync_val_compare_and_swap(&authenticated_ptr, 1, 1);


  __sync_lock_test_and_set(&authenticated_ptr, 1);

  __sync_lock_release(&authenticated_ptr);



int i = 0;

  __sync_fetch_and_add(&non_addr_discriminatedauthenticated_ptr, &i);
  __sync_fetch_and_sub(&non_addr_discriminatedauthenticated_ptr, &i);
  __sync_fetch_and_or(&non_addr_discriminatedauthenticated_ptr, &i);
  __sync_fetch_and_and(&non_addr_discriminatedauthenticated_ptr, &i);
  __sync_fetch_and_xor(&non_addr_discriminatedauthenticated_ptr, &i);

  __sync_add_and_fetch(&non_addr_discriminatedauthenticated_ptr, &i);
  __sync_sub_and_fetch(&non_addr_discriminatedauthenticated_ptr, &i);
  __sync_or_and_fetch(&non_addr_discriminatedauthenticated_ptr, &i);
  __sync_and_and_fetch(&non_addr_discriminatedauthenticated_ptr, &i);
  __sync_xor_and_fetch(&non_addr_discriminatedauthenticated_ptr, &i);

  __sync_bool_compare_and_swap(&non_addr_discriminatedauthenticated_ptr, &i, &i);
  __sync_val_compare_and_swap(&non_addr_discriminatedauthenticated_ptr, &i, &i);

  __sync_lock_test_and_set(&non_addr_discriminatedauthenticated_ptr, &i);
  __sync_lock_release(&non_addr_discriminatedauthenticated_ptr);
}
