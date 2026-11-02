// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// REQUIRES: powerpc-registered-target
// RUN: %clang_cc1 -triple powerpc64le -target-feature +htm -emit-llvm-only %s -verify

__attribute__((always_inline))
int test1(int *x) {
  *x = __builtin_ttest();
  return *x;
}

__attribute__((target("no-htm")))
int test2(int *x) {
  *x = test1(x); // expected-error {{always_inline function 'test1' requires target feature 'htm', but would be inlined into function 'test2' that is compiled without support for 'htm'}}
  return 0;
}
