// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang -target systemz-unknown-unknown -x c -E -dM -o - %s | FileCheck -match-full-lines %s
// RUN: %clang -target s390x-unknown-unknown -x c -E -dM -o - %s | FileCheck -match-full-lines %s

// CHECK: #define __GCC_ASM_FLAG_OUTPUTS__ 1
