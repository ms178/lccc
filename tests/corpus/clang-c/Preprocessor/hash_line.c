// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  -w -w -w --c
// The 1 and # should not go on the same line.
// RUN: %clang_cc1 -E %s | FileCheck --strict-whitespace %s
// CHECK: {{^1$}}
// CHECK-NEXT: {{^      #$}}
// CHECK-NEXT: {{^2$}}
// CHECK-NEXT: {{^           #$}}

// RUN: %clang_cc1 -E -P -fminimize-whitespace %s | FileCheck --strict-whitespace %s --check-prefix=MINWS
// MINWS:  {{^}}1#2#{{$}}

#define EMPTY
#define IDENTITY(X) X
1
EMPTY #
2
IDENTITY() #
