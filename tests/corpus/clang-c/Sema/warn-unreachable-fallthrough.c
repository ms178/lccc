// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c23: --c23: --c23: --c23
// RUN: %clang_cc1 -fsyntax-only -verify -std=c2x -Wunreachable-code-fallthrough %s
// RUN: %clang_cc1 -fsyntax-only -verify -std=c2x -Wunreachable-code %s
// RUN: %clang_cc1 -fsyntax-only -verify=code -std=c2x -Wunreachable-code -Wno-unreachable-code-fallthrough %s
// RUN: %clang_cc1 -fsyntax-only -verify -std=c2x -Wno-unreachable-code -Wunreachable-code-fallthrough %s

int n;
void f(void){
     switch (n){
         [[fallthrough]]; // expected-warning{{fallthrough annotation in unreachable code}}
                          // code-warning@-1{{never be executed}}
         case 1:;
     }
}
