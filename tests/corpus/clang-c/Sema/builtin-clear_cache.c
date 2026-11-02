// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c --target linux_aarch64
// RUN: %clang_cc1 -triple armv7-none-linux-gnu -fsyntax-only -verify %s
// RUN: %clang_cc1 -triple aarch64-none-linux-gnu -fsyntax-only -verify %s
// RUN: %clang_cc1 -triple arm64-none-linux-gnu -fsyntax-only -verify %s
// expected-no-diagnostics

void __clear_cache(void *a, void *b) {}
