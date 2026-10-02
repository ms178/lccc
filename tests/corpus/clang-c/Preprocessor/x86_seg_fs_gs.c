// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang -target i386-unknown-unknown -x c -E -dM -o - %s | FileCheck -match-full-lines %s
// RUN: %clang -target x86_64-unknown-unknown -x c -E -dM -o - %s | FileCheck -match-full-lines %s

// CHECK: #define __SEG_FS 1
// CHECK: #define __SEG_GS 1
// CHECK: #define __seg_fs __attribute__((address_space(257)))
// CHECK: #define __seg_gs __attribute__((address_space(256)))
