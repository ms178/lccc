// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 %s -fsyntax-only -verify
// PR4290

// The following declaration is not compatible with vfprintf(), but make
// sure this isn't an error: autoconf expects this to build.
char vfprintf(); // expected-warning {{declaration of built-in function 'vfprintf'}}
