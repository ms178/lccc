// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  -w --c
// RUN: %clang_cc1 -E %s | FileCheck -strict-whitespace %s

#define R_PAREN ) 

#define FUNC(a) a 

static int glob = (1 + FUNC(1 R_PAREN ); 

// CHECK: static int glob = (1 + 1 );

