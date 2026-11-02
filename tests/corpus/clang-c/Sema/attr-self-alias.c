// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -triple x86_64-pc-linux  -verify -emit-llvm-only %s

int self_alias(void) __attribute__((weak, alias("self_alias"))); // expected-error {{alias definition is part of a cycle}}

