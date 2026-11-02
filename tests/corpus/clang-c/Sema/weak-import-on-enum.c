// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1  -fsyntax-only -verify -triple x86_64-apple-darwin %s
// RUN: %clang_cc1 -triple i386-apple-darwin9 -fsyntax-only -verify %s
// expected-no-diagnostics

enum __attribute__((deprecated)) __attribute__((weak_import)) A {
    a0
};

