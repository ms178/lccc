// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -E -dM -ffreestanding -triple=arm64e-apple-ios < /dev/null | FileCheck %s

// CHECK: #define __ARM64_ARCH_8__ 1
// CHECK: #define __arm64__ 1
// CHECK: #define __arm64e__ 1
