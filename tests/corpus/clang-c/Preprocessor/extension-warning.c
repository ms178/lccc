// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify -pedantic %s

// The preprocessor shouldn't warn about extensions within macro bodies that
// aren't expanded.
#define TY typeof
#define TY1 typeof(1)

// But we should warn here
TY1 x; // expected-warning {{extension}}
TY(1) x; // FIXME: And we should warn here

// Note: this warning intentionally doesn't trigger on keywords like
// __attribute; the standard allows implementation-defined extensions
// prefixed with "__".
// Current list of keywords this can trigger on:
// inline, restrict, asm, typeof, _asm

void whatever(void) {}
