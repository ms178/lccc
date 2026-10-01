// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -E -dM -triple i586-intel-elfiamcu | FileCheck %s

// CHECK: #define __USER_LABEL_PREFIX__ {{$}}
// CHECK: #define __WINT_TYPE__ unsigned int
// CHECK: #define __iamcu
// CHECK: #define __iamcu__

