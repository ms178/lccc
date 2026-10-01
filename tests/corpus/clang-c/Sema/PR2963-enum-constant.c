// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 %s -verify -pedantic -fsyntax-only

typedef short short_fixed;

enum
{
        // 8.8 short_fixed
        SHORT_FIXED_FRACTIONAL_BITS= 8,
        SHORT_FIXED_ONE= 1<<SHORT_FIXED_FRACTIONAL_BITS
};

#define FLOAT_TO_SHORT_FIXED(f) ((short_fixed)((f)*SHORT_FIXED_ONE))

enum
{
        SOME_VALUE= FLOAT_TO_SHORT_FIXED(0.1) // expected-warning{{expression is not an integer constant expression}}
};
