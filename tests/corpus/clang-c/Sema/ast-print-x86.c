// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -triple i686-elf %s -ast-print | FileCheck %s

// REQUIRES: x86-registered-target

void assembly(void) {
  int added;
  // CHECK: asm volatile ("addl %%ebx,%%eax" : "=a" (added) : "a" (1), "b" (2));
  __asm__ __volatile__("addl %%ebx,%%eax" : "=a" (added) : "a" (1), "b" (2) );
}
