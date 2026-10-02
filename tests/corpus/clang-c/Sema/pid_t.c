// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -triple i586-pc-haiku -fsyntax-only -verify %s
// RUN: %clang_cc1 -triple i686-pc-linux -fsyntax-only -verify %s

// expected-no-diagnostics

#ifdef __HAIKU__
typedef signed long pid_t;
#else
typedef signed int pid_t;
#endif
pid_t	 vfork(void);