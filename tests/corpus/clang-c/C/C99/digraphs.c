// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
# 1 "C/C99/digraphs.c"
# 1 "<built-in>" 1
# 1 "<built-in>" 3
# 412 "<built-in>" 3
# 1 "<command line>" 1
# 1 "<built-in>" 2
# 1 "C/C99/digraphs.c" 2
# 13 "C/C99/digraphs.c"
# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/iso646.h" 1
# 14 "C/C99/digraphs.c" 2






_Static_assert((1 && 1) == (1 && 1), "");
# 30 "C/C99/digraphs.c"
_Static_assert((1 & 3) == (1 & 3), "");





_Static_assert((1 | 2) == (1 | 2), "");





_Static_assert((~ 0) == (~0), "");





_Static_assert((! 12) == (!12), "");





_Static_assert((0 != 12) == (0 != 12), "");







_Static_assert((0 || 12) == (0 || 12), "");
# 73 "C/C99/digraphs.c"
_Static_assert((1 ^ 3) == (1 ^ 3), "");
# 84 "C/C99/digraphs.c"
void foobar(int (*array)<: 0 :>);
void foobar(int (*array)[0]) {}



_Static_assert(__builtin_strcmp("testing", "testing") == 0, "");
