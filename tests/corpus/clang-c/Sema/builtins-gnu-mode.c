// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c99
// RUN: %clang_cc1 -fsyntax-only -verify -std=c99 %s
// PR16138
// expected-no-diagnostics

int alloca;
int stpcpy;
int stpncpy;
int strdup;
int strndup;
int index;
int rindex;
int bzero;
int strcasecmp;
int strncasecmp;
int _exit;
int _longjmp;
int siglongjmp;
int strlcpy;
int strlcat;
