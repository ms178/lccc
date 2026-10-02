// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  -w --c
// RUN: %clang_cc1 -E %s | FileCheck %s --match-full-lines --strict-whitespace
// CHECK: # define X 3

#define H # 
 #define D define 
 
 #define DEFINE(a, b) H D a b 
 
 DEFINE(X, 3) 

