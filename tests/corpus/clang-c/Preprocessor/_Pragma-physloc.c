// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  -w --c
// RUN: %clang_cc1 -E %s | FileCheck --strict-whitespace %s
// CHECK: {{^}}#pragma x y z{{$}}
// CHECK: {{^}}#pragma a b c{{$}}

_Pragma("x y z")
_Pragma("a b c")

