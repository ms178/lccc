// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c --target linux_aarch64
// RUN: %clang_cc1 -triple arm64e-apple-darwin -fsyntax-only -verify %s -fptrauth-indirect-gotos
// RUN: %clang_cc1 -triple aarch64-linux-gnu   -fsyntax-only -verify %s -fptrauth-indirect-gotos

int f() {
  static void *addrs[] = { &&l1, &&l2 };

  static int diffs[] = {
    &&l1 - &&l1, // expected-error{{subtraction of address-of-label expressions is not supported with ptrauth indirect gotos}}
    &&l1 - &&l2 // expected-error{{subtraction of address-of-label expressions is not supported with ptrauth indirect gotos}}
  };

  int diff_32 = &&l1 - &&l2; // expected-error{{subtraction of address-of-label expressions is not supported with ptrauth indirect gotos}}
  goto *(&&l1 + diff_32); // expected-error{{addition of address-of-label expressions is not supported with ptrauth indirect gotos}}

l1:
  return 0;
l2:
  return 1;
}
