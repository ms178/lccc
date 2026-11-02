// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify %s

#define __pso __attribute__((preserve_static_offset))

struct foo { int a; } __pso; // expected-warning{{unknown attribute}}
union quux { int a; } __pso; // expected-warning{{unknown attribute}}
