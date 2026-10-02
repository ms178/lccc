// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
# 1 "C/C2y/n3364.c"
# 1 "<built-in>" 1
# 1 "<built-in>" 3
# 412 "<built-in>" 3
# 1 "<command line>" 1
# 1 "<built-in>" 2
# 1 "C/C2y/n3364.c" 2
# 23 "C/C2y/n3364.c"
float f1 = __builtin_nansf("");
float f2 = +__builtin_nansf("");
float f3 = -__builtin_nansf("");




double d1 = __builtin_nans("");
double d2 = +__builtin_nans("");
double d3 = -__builtin_nans("");




long double ld1 = __builtin_nansl("");
long double ld2 = +__builtin_nansl("");
long double ld3 = -__builtin_nansl("");
