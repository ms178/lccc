// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -E %s -Wall -verify
// RUN: %clang_cc1 -Eonly %s -Wall -verify
// RUN: %clang -M -Wall %s -Xclang -verify
// RUN: %clang -E -frewrite-includes %s -Wall -Xclang -verify
// RUN: %clang -E -dD -dM %s -Wall -Xclang -verify
// expected-no-diagnostics

#pragma GCC visibility push (default)
#pragma weak
#pragma this_pragma_does_not_exist
