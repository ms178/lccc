// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  -Dfoo='bar\' --c
// RUN: %clang_cc1 -E %s -Dfoo='bar\' | FileCheck %s
// CHECK: TTA bar\ TTB
TTA foo TTB
