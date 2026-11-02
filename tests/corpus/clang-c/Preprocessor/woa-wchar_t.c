// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  -w -w --c
// RUN: %clang_cc1 -dM -triple armv7-windows -E %s | FileCheck %s
// RUN: %clang_cc1 -dM -fno-signed-char -triple armv7-windows -E %s | FileCheck %s

// CHECK: #define __WCHAR_TYPE__ unsigned short

