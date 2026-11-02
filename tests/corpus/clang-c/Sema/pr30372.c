// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  -w --ms_extensions --ms_compatibility --c: --c++11 --c++
// REQUIRES: x86-registered-target
// RUN: %clang_cc1 %s -triple i386-pc-windows-msvc18.0.0 -disable-free -fms-volatile -fms-extensions -fms-compatibility -fms-compatibility-version=18 -std=c++11 -x c++

// Check that the parser catching an 'error' from forward declaration of "location" does not lexer out it's subsequent declaration.

void foo() {
  __asm {
    jl         location
 location:
    ret
  }
}
