// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c23 --ms_compatibility -w
// RUN: %clang_cc1 -std=c23 -fdefer-ts -fms-compatibility -triple x86_64-windows-msvc -fsyntax-only -verify %s

void f() {
  __try {
    _Defer {
      __leave; // expected-error {{cannot __leave a defer statement}}
    }
  } __finally {}

  __try {
    _Defer {
      __try {
        __leave;
      } __finally {}
    }
  } __finally {}
}
