// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -target-feature +altivec -fsyntax-only %s -triple powerpc64-ibm-aix-xcoff
// RUN: %clang_cc1 -target-feature +altivec -fsyntax-only %s -triple powerpc64le-ibm-linux-gnu
// RUN: %clang_cc1 -target-feature +altivec -fsyntax-only %s -triple powerpc64-linux-gnu
// RUN: %clang_cc1 -target-feature +altivec -fsyntax-only %s -triple powerpc-ibm-aix-xcoff
// RUN: %clang_cc1 -target-feature +altivec -fsyntax-only %s -triple powerpc-linux-gnu

typedef int vector;

void test(void) {
  vector unsigned int v = {0};
}
