// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -E %s | grep 'noexp: foo y'
// RUN: %clang_cc1 -E %s | grep 'expand: abc'
// RUN: %clang_cc1 -E %s | grep 'noexp2: foo nonexp'
// RUN: %clang_cc1 -E %s | grep 'expand2: abc'

#define A foo
#define foo() abc
#define X A y

// This should not expand to abc, because the foo macro isn't followed by (.
noexp: X


// This should expand to abc.
#undef X
#define X A ()
expand: X


// This should be 'foo nonexp'
noexp2: A nonexp

// This should expand
expand2: A (
)


