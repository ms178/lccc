// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -E %s | FileCheck %s

#pragma clang __debug parser_crash
#pragma clang __debug dump Test

// CHECK: #pragma clang __debug parser_crash
// FIXME: The dump parameter is dropped.
// CHECK: #pragma clang __debug dump{{$}}
