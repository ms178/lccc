// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 %s -fsyntax-only

// C99 6.7.2.3p11

// mutually recursive structs
struct s1 { struct s2 *A; };
struct s2 { struct s1 *B; };

// both types are complete now.
struct s1 a;
struct s2 b;
