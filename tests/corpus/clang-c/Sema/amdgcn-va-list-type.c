// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 %s -triple amdgcn-amd-amdhsa -fsyntax-only -verify
// RUN: %clang_cc1 %s -triple spirv64-amd-amdhsa -fsyntax-only -verify

// expected-no-diagnostics

typedef char* va_list;

void foo(const char* f, ...) {
    int r;
    va_list args;
    __builtin_va_start(args, f);
    __builtin_va_end(args);
}