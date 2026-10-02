// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
/* RUN: %clang -E -C -P %s | FileCheck --strict-whitespace %s
   PR2741
   comment */ 
y
// CHECK: {{^}}   comment */{{$}}
// CHECK-NEXT: {{^}}y{{$}}

