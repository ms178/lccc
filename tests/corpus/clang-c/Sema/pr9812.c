// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify %s

#define bool _Bool
int main(int argc, char** argv)
{
    bool signed;  // expected-error {{'bool' cannot be signed or unsigned}} expected-warning {{declaration does not declare anything}}

    return 0;
}
#undef bool

typedef int bool;

int test2(int argc, char** argv)
{
    bool signed; // expected-error {{'type-name' cannot be signed or unsigned}} expected-warning {{declaration does not declare anything}}
    _Bool signed; // expected-error {{'_Bool' cannot be signed or unsigned}} expected-warning {{declaration does not declare anything}}

    return 0;
}

