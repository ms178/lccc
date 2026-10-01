// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -Wno-gnu-designator -verify %s
// expected-no-diagnostics
struct { int x, y, z[12]; } value = { x:17, .z [3 ... 5] = 7 };
