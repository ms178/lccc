// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 %s -E | grep 'a: 2\*f(9)'
// RUN: %clang_cc1 %s -E | grep 'b: 2\*9\*g'

#define f(a) a*g 
#define g f 
a: f(2)(9) 

#undef f
#undef g

#define f(a) a*g 
#define g(a) f(a) 

b: f(2)(9)

