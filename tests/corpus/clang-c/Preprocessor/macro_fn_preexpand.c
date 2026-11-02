// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 %s -E | grep 'pre: 1 1 X'
// RUN: %clang_cc1 %s -E | grep 'nopre: 1A(X)'

/* Preexpansion of argument. */
#define A(X) 1 X
pre: A(A(X))

/* The ## operator disables preexpansion. */
#undef A
#define A(X) 1 ## X
nopre: A(A(X))

