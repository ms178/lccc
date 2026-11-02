// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -triple=i386-none-none -fsyntax-only -verify %s
// RUN: %clang_cc1 -triple=x86_64-none-none -fsyntax-only -verify %s

// Check that we can undefine triple-specific defines without warning
// expected-no-diagnostics
#undef __i386
#undef __i386__
#undef i386
#undef __amd64
#undef __amd64__
#undef __x86_64
#undef __x86_64__
