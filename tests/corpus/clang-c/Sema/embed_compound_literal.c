// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c23
// RUN: %clang_cc1 -std=c23 -fsyntax-only -verify %s
// RUN: %clang_cc1 -fsyntax-only -verify -x c++ -Wno-c23-extensions %s
// expected-no-diagnostics

char *p1 = (char[]){
#embed __FILE__
};

int *p2 = (int[]){
#embed __FILE__
};

int *p3 = (int[30]){
#embed __FILE__ limit(30)
};
