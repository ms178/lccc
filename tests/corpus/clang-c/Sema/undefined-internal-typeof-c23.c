// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c23
// RUN: %clang_cc1 -fsyntax-only -verify %s -std=c23 -pedantic-errors

// expected-no-diagnostics

static int f(void);

int main(void)
{
    typeof(&f) x;
}
