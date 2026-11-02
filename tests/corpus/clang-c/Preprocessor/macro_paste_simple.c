// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 %s -E | FileCheck %s

#define FOO bar ## baz ## 123

// CHECK: A: barbaz123
A: FOO

// PR9981
#define M1(A) A
#define M2(X) X
B: M1(M2(##))

// CHECK: B: ##

