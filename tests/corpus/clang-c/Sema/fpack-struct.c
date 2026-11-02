// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  %-DEXPECTED_STRUCT_SIZE=5 %-DEXPECTED_STRUCT_SIZE=6 --c
// RUN: %clang_cc1 -DEXPECTED_STRUCT_SIZE=5 -fpack-struct=1 %s
// RUN: %clang_cc1 -DEXPECTED_STRUCT_SIZE=6 -fpack-struct=2 %s

struct s0 {
       int x;
       char c;
};

int t0[sizeof(struct s0) == EXPECTED_STRUCT_SIZE ?: -1];
