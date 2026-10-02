// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -Wunused-value -verify %s
int main(void) {
    int a;
    int b;
    a ? : b; //expected-warning{{expression result unused}}
    a ? a : b; //expected-warning{{expression result unused}}
    a ? : ++b;
    a ? a : ++b;
    ++a ? : b; //expected-warning{{expression result unused}}
    ++a ? a : b; //expected-warning{{expression result unused}}
    ++a ? : ++b;
    ++a ? a : ++b;
    return 0;
};

