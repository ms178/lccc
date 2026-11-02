// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
/// Assert the fixed point precision macros according to ISO/IEC TR 18037:2008 7.18a.3 are not
/// defined when -ffixed-point is not provided.

// RUN: %clang_cc1 -triple=x86_64 -E -dM -x c < /dev/null | FileCheck -match-full-lines %s
// RUN: %clang_cc1 -triple=x86_64 -E -dM -x c++ < /dev/null | FileCheck -match-full-lines %s

// CHECK-NOT:#define __SFRACT_FBIT__ 7
