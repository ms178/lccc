// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c23
// RUN: %clang_cc1 %s -std=c2x -Wpre-c2x-compat -pedantic -fsyntax-only -verify

int digit_seps = 123'456; // expected-warning {{digit separators are incompatible with C standards before C23}}
unsigned char u8_char = u8'x'; // expected-warning {{unicode literals are incompatible with C standards before C23}}
