// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 %s -fsyntax-only -verify
// expected-no-diagnostics

typedef union   {
  union wait *__uptr;
  int *__iptr;
} __WAIT_STATUS __attribute__ ((__transparent_union__));

extern int wait (__WAIT_STATUS __stat_loc);

void fastcgi_cleanup(void) {
  int status = 0;
  wait(&status);
}

