// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
# 1 "Sema/block-printf-attribute-1.c"
# 1 "<built-in>" 1
# 1 "<built-in>" 3
# 412 "<built-in>" 3
# 1 "<command line>" 1
# 1 "<built-in>" 2
# 1 "Sema/block-printf-attribute-1.c" 2


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
# 4 "Sema/block-printf-attribute-1.c" 2

int main(void) {
  void (^b) (int arg, const char * format, ...) __attribute__ ((__format__ (__printf__, 1, 3))) =
    ^ __attribute__ ((__format__ (__printf__, 1, 3))) (int arg, const char * format, ...) {};

  void (^z) (int arg, const char * format, ...) __attribute__ ((__format__ (__printf__, 2, 3))) = ^ __attribute__ ((__format__ (__printf__, 2, 3))) (int arg, const char * format, ...) {};

  z(1, "%s", 1);
  z(1, "%s", "HELLO");
}

void multi_attr(va_list ap, int *x, long *y) {

  void (^vprintf_scanf) (const char *, va_list, const char *, ...) __attribute__((__format__(__printf__, 1, 0))) __attribute__((__format__(__scanf__, 3, 4))) =
  ^ __attribute__((__format__(__printf__, 1, 0))) __attribute__((__format__(__scanf__, 3, 4))) (const char *str, va_list args, const char *fmt, ...) {};

  vprintf_scanf("%", ap, "%d");
}
