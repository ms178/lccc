// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  -w -w -w -w -w --c
       foo
// RUN: %clang_cc1 -E %s | FileCheck -strict-whitespace %s
// RUN: %clang_cc1 -E -fminimize-whitespace %s | FileCheck -strict-whitespace %s --check-prefix=MINCOL
// RUN: %clang_cc1 -E -fminimize-whitespace -P %s | FileCheck -strict-whitespace %s --check-prefix=MINWS
       bar

// CHECK: {{^       }}foo
// CHECK: {{^       }}bar

// MINCOL: {{^}}foo
// MINCOL: {{^}}bar

// MINWS: {{^}}foo bar

