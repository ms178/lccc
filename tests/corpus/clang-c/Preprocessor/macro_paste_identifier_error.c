// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --ms_extensions --ms_extensions --ms_extensions -w --c
// RUN: %clang_cc1 -fms-extensions -Wno-invalid-token-paste %s -verify
// RUN: %clang_cc1 -E -fms-extensions -Wno-invalid-token-paste %s | FileCheck %s
// RUN: %clang_cc1 -E -fms-extensions -Wno-invalid-token-paste -x assembler-with-cpp %s | FileCheck %s
// expected-no-diagnostics

#define foo a ## b ## = 0
int foo;
// CHECK: int ab = 0;
