// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -triple=powerpc64-unknown-linux-gnu -target-cpu pwr10 \
// RUN:            -target-feature +vsx -target-feature +power10-vector \
// RUN:            -fsyntax-only -verify %s
// RUN: %clang_cc1 -triple=powerpc64le-unknown-linux-gnu -target-cpu pwr10 \
// RUN:            -target-feature +power10-vector -fsyntax-only -verify %s
// expected-no-diagnostics

// Test legitimate uses of 'vector bool __int128' with VSX.
__vector bool __int128 v1_bi128;
__vector __bool __int128 v2_bi128;
vector bool __int128 v3_bi128;
vector __bool __int128 v4_bi128;
