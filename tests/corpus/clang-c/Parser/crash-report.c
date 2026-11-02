// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: not --crash %clang_cc1 %s 2>&1 | FileCheck %s
// REQUIRES: crash-recovery

// Stack traces also require back traces.
// REQUIRES: backtrace

#prag\
ma clang __debug crash

// CHECK: prag\
// CHECK-NEXT: ma

