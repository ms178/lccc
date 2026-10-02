// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -emit-llvm -triple i386-linux-gnu -o %t %s
// RUN: FileCheck --input-file=%t %s
// PR10392

#define push(foo) push(default)
#pragma GCC visibility push(hidden)

int v1;
// CHECK: @v1 = hidden global i32 0, align 4

#pragma GCC visibility pop

int v2;
// CHECK: @v2 ={{.*}} global i32 0, align 4

_Pragma("GCC visibility push(hidden)");

int v3;
// CHECK: @v3 = hidden global i32 0, align 4
