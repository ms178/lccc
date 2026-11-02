// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 %s -fsyntax-only -verify -fblocks

typedef short SHORT;

void f0(void) {
  (void) ^{
    if (1)
      return (float)1.0;
    else if (2)
      return (double)2.0; // expected-error {{return type 'double' must match previous return type 'float' when block literal has}}
    else
      return (SHORT)3; // expected-error {{return type 'SHORT' (aka 'short') must match previous return type 'float' when}}
  };
}
