// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -E -dD %s | FileCheck %s
// PR7818

// CHECK: # 1 "{{.+}}.c"
#define X 3
// CHECK: #define X 3
#undef X
// CHECK: #undef X
