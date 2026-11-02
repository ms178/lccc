// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify=WARNING-ON %s
// RUN: %clang_cc1 -fsyntax-only -Wno-format-insufficient-args -verify=WARNING-OFF %s


int printf(const char * format, ...);

int main(void) {
  int patatino = 42;
  printf("%i %i", patatino); // WARNING-ON-warning {{more '%' conversions than data arguments}}
  // WARNING-OFF-no-diagnostics
}
