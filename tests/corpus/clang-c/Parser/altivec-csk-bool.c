// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -triple powerpc64-unknown-linux-gnu -target-feature +altivec -fsyntax-only %s
// RUN: %clang_cc1 -triple powerpc64le-unknown-linux-gnu -target-feature +altivec -fsyntax-only %s

// PR16456: Verify that bool, true, false are treated as context-sensitive
// keywords (and therefore available for use as identifiers) when in
// Altivec mode.

typedef enum {
  false_value = 0,
  true_value = 1
} bool;

#define true true_value
#define false false_value

