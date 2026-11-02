// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -E %s | FileCheck %s
#define hash_hash # ## # 
#define mkstr(a) # a 
#define in_between(a) mkstr(a) 
#define join(c, d) in_between(c hash_hash d) 
// CHECK: "x ## y";
join(x, y);

#define FOO(x) A x B
// CHECK: A ## B;
FOO(##);
