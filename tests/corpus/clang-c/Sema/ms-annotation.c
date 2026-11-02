// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  -w --ms_extensions --c: --c++11 --c++ -w --ms_extensions -w --ms_extensions
// RUN: %clang_cc1 -triple i686-windows %s -verify -fms-extensions
// RUN: %clang_cc1 -x c++ -std=c++11 -triple i686-windows %s -verify -fms-extensions
// RUN: %clang_cc1 -x c++ -std=c++14 -triple i686-windows %s -verify -fms-extensions

void test1(void) {
  __annotation(); // expected-error {{too few arguments to function call, expected at least 1, have 0}}
  __annotation(1); // expected-error {{must be wide string constants}}
  __annotation(L"a1");
  __annotation(L"a1", L"a2");
  __annotation(L"a1", L"a2", 42); // expected-error {{must be wide string constants}}
  __annotation(L"a1", L"a2", L"a3");
  __annotation(L"multi " L"part " L"string");
}
