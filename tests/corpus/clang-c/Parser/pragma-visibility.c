// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify %s

#pragma GCC visibility foo // expected-warning{{expected identifier in '#pragma visibility' - ignored}}
#pragma GCC visibility pop foo // expected-warning{{extra tokens at end of '#pragma visibility' - ignored}}
#pragma GCC visibility push // expected-warning{{missing '(' after '#pragma visibility'}}
#pragma GCC visibility push( // expected-warning{{expected identifier in '#pragma visibility' - ignored}}
#pragma GCC visibility push(hidden // expected-warning{{missing ')' after '#pragma visibility' - ignoring}}
#pragma GCC visibility push(hidden)
#pragma GCC visibility pop
