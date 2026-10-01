// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -E -x c %s | FileCheck %s
// RUN: %clang_cc1 -x c -fsyntax-only %s -verify
// RUN: %clang_cc1 -x cpp-output -fsyntax-only -verify %s
// expected-no-diagnostics

// The preprocessor does not expand macro-identifiers in #pragma directives.
// When we preprocess & parse the code, clang expands the macros in directives.
// When we parse already preprocessed code, clang still has to expand the
// macros in the directives.
// This means that we're not always able to parse the preprocessor's output
// without preserving the definitions (-dD).

#define FACTOR 4

void foo() {
    // CHECK: #pragma unroll FACTOR
    #pragma unroll FACTOR
    for(;;) {
    }
    return;
}
