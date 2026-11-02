// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -E -x c %s | FileCheck %s --check-prefixes=EXPANDED
// RUN: %clang_cc1 -E -x cpp-output %s | FileCheck %s --check-prefixes=NOT-EXPANDED

// EXPANDED: void __attribute__((__attribute__((always_inline)))) foo()
// NOT-EXPANDED: void __attribute__((always_inline)) foo()

#define always_inline __attribute__((always_inline))
void __attribute__((always_inline)) foo() {
    return 4;
}
