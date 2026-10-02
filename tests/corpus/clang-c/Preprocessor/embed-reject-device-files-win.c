// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c23
// RUN: %clang_cc1 -std=c23 %s -fsyntax-only -verify
// REQUIRES: system-windows


int null[] = {
#embed "NUL" limit(1) //expected-error {{device files are not yet supported by '#embed' directive}}
};
