// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -fsyntax-only %s 2>&1 | FileCheck %s
// RUN: %clang_cc1 -fsyntax-only -fapple-pragma-pack %s 2>&1 | FileCheck -check-prefix=CHECK-APPLE %s

#pragma pack(push,1)
#pragma pack(2)
#pragma pack()
#pragma pack(show)

// CHECK: pack(show) == 8
// CHECK-APPLE: pack(show) == 1
