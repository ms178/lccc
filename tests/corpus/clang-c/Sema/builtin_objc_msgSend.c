// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 %s -fsyntax-only -verify
// expected-no-diagnostics

typedef struct objc_class *Class;
typedef struct objc_object {
    Class isa;
} *id;


typedef struct objc_selector *SEL;
extern id objc_msgSend(id self, SEL op, ...);

