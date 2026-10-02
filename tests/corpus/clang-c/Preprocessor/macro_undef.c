// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  -Dfoo=1 --c
// RUN: %clang_cc1 -dM -undef -Dfoo=1 -E %s | FileCheck %s

// CHECK-NOT: #define __clang__
// CHECK: #define foo 1
