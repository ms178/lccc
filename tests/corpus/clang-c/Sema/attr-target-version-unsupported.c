// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -triple x86_64-unknown-unknown -fsyntax-only -verify %s

//expected-warning@+1 {{unknown attribute 'target_version' ignored}}
int __attribute__((target_version("aes"))) foo(void) { return 3; }
