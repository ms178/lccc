// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  -w --c
// RUN: %clang_cc1 -E %s | FileCheck --strict-whitespace %s
#define FOO __LINE__

  FOO
// CHECK: {{^}}  4{{$}}

// PR3579 - This should expand to the __LINE__ of the ')' not of the X.

#define X() __LINE__

A X(

)
// CHECK: {{^}}A 13{{$}}

