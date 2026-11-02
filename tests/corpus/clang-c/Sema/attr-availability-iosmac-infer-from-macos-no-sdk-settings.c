// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 "-triple" "x86_64-apple-ios13.1-macabi" -fsyntax-only -verify %s

void f0(void) __attribute__((availability(macOS, introduced = 10.11)));
// expected-warning@-1 {{macOS availability is ignored without a valid 'SDKSettings.json' in the SDK}}
void f1(void) __attribute__((availability(macOS, introduced = 10.15)));
