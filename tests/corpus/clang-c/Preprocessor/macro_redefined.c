// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  -DCLI_MACRO=1 -DWMACRO_REDEFINED -DCLI_MACRO=1 --c
// RUN: %clang_cc1 %s -Eonly -verify -Wno-all -Wmacro-redefined -DCLI_MACRO=1 -DWMACRO_REDEFINED
// RUN: %clang_cc1 %s -Eonly -verify -Wno-all -Wno-macro-redefined -DCLI_MACRO=1

#ifndef WMACRO_REDEFINED
// expected-no-diagnostics
#endif

#ifdef WMACRO_REDEFINED
// expected-note@1 {{previous definition is here}}
// expected-warning@+2 {{macro redefined}}
#endif
#define CLI_MACRO

#ifdef WMACRO_REDEFINED
// expected-note@+3 {{previous definition is here}}
// expected-warning@+3 {{macro redefined}}
#endif
#define REGULAR_MACRO
#define REGULAR_MACRO 1
