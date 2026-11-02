// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -E %s | grep 'FUNC (3 +1);'

#define F(a) a 
#define FUNC(a) (a+1) 

F(FUNC) FUNC (3); /* final token sequence is FUNC(3+1) */ 

