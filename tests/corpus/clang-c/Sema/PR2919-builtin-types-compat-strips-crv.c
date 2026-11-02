// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -fsyntax-only %s
// RUN: %clang_cc1 -fsyntax-only %s -fexperimental-new-constant-interpreter

typedef struct foo T0;
typedef const struct foo T1;

int a0[__builtin_types_compatible_p(T0,
                                    const T1) ? 1 : -1];
