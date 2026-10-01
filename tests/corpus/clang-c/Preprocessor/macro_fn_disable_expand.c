// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 %s -E | FileCheck %s

#define foo(x) bar x
foo(foo) (2)
// CHECK: bar foo (2)

#define m(a) a(w)
#define w ABCD
m(m)
// CHECK: m(ABCD)



// PR4438, PR5163

// We should get '42' in the argument list for gcc compatibility.
#define A 1
#define B 2
#define C(x) (x + 1)

X: C(
#ifdef A
#if A == 1
#if B
    42
#endif
#endif
#endif
    )
// CHECK: X: (42 + 1)
