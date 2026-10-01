// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -triple powerpc64-unknown-linux-gnu -E -x c %s
// RUN: %clang_cc1 -triple powerpc64-unknown-linux-gnu -E -fno-signed-char -x c %s

#if (L'\0' - 1 > 0)
# error "Unexpected expression evaluation result"
#endif
