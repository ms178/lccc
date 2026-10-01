// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -triple arm-unknown-unknown -target-abi apcs-gnu -fsyntax-only -verify %s
// expected-no-diagnostics

struct s0 { double f0; int f1; };
char chk0[__alignof__(struct s0) == 4 ? 1 : -1]; 
