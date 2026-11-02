// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c99
// RUN: %clang_cc1 -fsyntax-only -W -Wall -Wno-deprecated-non-prototype -Werror -verify %s -std=c99
// expected-no-diagnostics

int f(int i __attribute__((__unused__)))
{
    return 0;
}
int g(i)
    int i __attribute__((__unused__));
{
    return 0;
}
