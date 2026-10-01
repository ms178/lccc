// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -triple x86_64-apple-darwin10 -fsyntax-only -verify %s
// RUN: %clang_cc1 -triple i386-pc-linux-gnu -fsyntax-only -verify %s

/* expected-error {{mac68k alignment pragma is not supported}} */ #pragma options align=mac68k
