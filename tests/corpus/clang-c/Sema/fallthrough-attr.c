// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options: : --c89 --strict_gnu: --c99 --strict_gnu: --c99: --c11: --c23 -DC2X
// RUN: %clang_cc1 -fsyntax-only -std=gnu89 -verify -Wimplicit-fallthrough %s
// RUN: %clang_cc1 -fsyntax-only -std=gnu99 -verify -Wimplicit-fallthrough %s
// RUN: %clang_cc1 -fsyntax-only -std=c99 -verify -Wimplicit-fallthrough %s
// RUN: %clang_cc1 -fsyntax-only -std=c11 -verify -Wimplicit-fallthrough %s
// RUN: %clang_cc1 -fsyntax-only -std=c2x -DC2X -verify -Wimplicit-fallthrough %s

int fallthrough_attribute_spelling(int n) {
  switch (n) {
  case 0:
    n++;
  case 1:
#if defined(C2X)
// expected-warning@-2{{unannotated fall-through between switch labels}} expected-note@-2{{insert '[[fallthrough]];' to silence this warning}} expected-note@-2{{insert 'break;' to avoid fall-through}}
#else
// expected-warning@-4{{unannotated fall-through between switch labels}} expected-note@-4{{insert '__attribute__((fallthrough));' to silence this warning}} expected-note@-4{{insert 'break;' to avoid fall-through}}
#endif
    n++;
    __attribute__((fallthrough));
  case 2:
    n++;
    break;
  }
  return n;
}
