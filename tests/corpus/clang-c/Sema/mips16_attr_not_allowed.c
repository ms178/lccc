// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -triple x86_64-unknown-linux-gnu -fsyntax-only -verify %s

void __attribute__((nomips16)) foo32(void); // expected-warning {{unknown attribute 'nomips16' ignored}}
void __attribute__((mips16)) foo16(void); // expected-warning {{unknown attribute 'mips16' ignored}}



