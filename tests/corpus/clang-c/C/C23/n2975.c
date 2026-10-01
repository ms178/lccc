// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c23
# 1 "C/C23/n2975.c"
# 1 "<built-in>" 1
# 1 "<built-in>" 3
# 444 "<built-in>" 3
# 1 "<command line>" 1
# 1 "<built-in>" 2
# 1 "C/C23/n2975.c" 2






# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdarg.h" 1
# 47 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdarg.h"
# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stdarg_header_macro.h" 1
# 48 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdarg.h" 2



# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stdarg___gnuc_va_list.h" 1
# 12 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stdarg___gnuc_va_list.h"
typedef __builtin_va_list __gnuc_va_list;
# 52 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdarg.h" 2




# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stdarg_va_list.h" 1
# 12 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stdarg_va_list.h"
typedef __builtin_va_list va_list;
# 57 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdarg.h" 2




# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stdarg_va_arg.h" 1
# 62 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdarg.h" 2




# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stdarg___va_copy.h" 1
# 67 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdarg.h" 2




# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stdarg_va_copy.h" 1
# 72 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdarg.h" 2
# 8 "C/C23/n2975.c" 2

void func(...) {

  va_list list;
  __builtin_c23_va_start(list);
  __builtin_va_end(list);

  __builtin_c23_va_start(list, 1, 2);
  __builtin_va_end(list);


  __builtin_va_start(list);


  _Static_assert(__builtin_types_compatible_p(__typeof__(__builtin_c23_va_start(list)), void), "");
  _Static_assert(__builtin_types_compatible_p(__typeof__(__builtin_va_start(list, 0)), void), "");
}



typedef void (*fp)(...);


void diag(int a, int b, ...) {
  va_list list;
  __builtin_c23_va_start(list, a);


  __builtin_va_start(list, a);
  __builtin_va_end(list);
}

void foo(int a...);

void use(void) {


  func(1, '2', 3.0, "4");
  func();


  fp local = func;


  fp other_local = diag;
}



int va_fn(...);


typeof(int(...))*fn_ptr = &va_fn;



int i = _Generic(typeof(va_fn), int(...):1);
