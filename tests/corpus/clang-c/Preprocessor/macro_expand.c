// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  -w --c
// RUN: %clang_cc1 -E %s | FileCheck --strict-whitespace %s

#define X() Y
#define Y() X

A: X()()()
// CHECK: {{^}}A: Y{{$}}

// PR3927
#define f(x) h(x
#define for(x) h(x
#define h(x) x()
B: f(f))
C: for(for))

// CHECK: {{^}}B: f(){{$}}
// CHECK: {{^}}C: for(){{$}}

#define f(x,y...) y
f()

// CHECK: #pragma omp parallel for
#define FOO parallel
#define Streaming _Pragma("omp FOO for")
Streaming

