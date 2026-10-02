// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -E -fopenacc %s | FileCheck %s --check-prefix=DEFAULT

// DEFAULT: OpenACC:202506:
OpenACC:_OPENACC:

// RUN: %clang_cc1 -E -dM -fopenacc %s | FileCheck %s --check-prefix=MACRO_PRINT_DEF
// MACRO_PRINT_DEF: #define _OPENACC 202506


