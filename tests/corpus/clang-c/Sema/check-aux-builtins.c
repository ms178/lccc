// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c --target linux_aarch64
// RUN: %clang_cc1 -fopenmp -fopenmp-is-target-device -triple aarch64 -aux-triple x86_64-linux-pc -fsyntax-only -verify %s

void func(void) {
  (void)__builtin_cpu_is("atom");
  __builtin_cpu_is("INVALID"); // expected-error{{invalid cpu name for builtin}}
}
