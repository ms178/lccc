// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// REQUIRES: hexagon-registered-target
// RUN: %clang_cc1 %s -triple hexagon -fsyntax-only -verify -target-cpu hexagonv65

// expected-no-diagnostics
unsigned builtin_needs_v60(unsigned Rs) {
  return __builtin_HEXAGON_S6_rol_i_r(Rs, 3);
}

unsigned long long builtin_needs_v62(unsigned Rs) {
  return __builtin_HEXAGON_S6_vsplatrbp(Rs);
}

unsigned builtin_needs_v65(unsigned long long Rss, unsigned long long Rtt) {
  return __builtin_HEXAGON_A6_vcmpbeq_notany(Rss, Rtt);
}
