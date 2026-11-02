// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -x c -triple bpf-pc-linux-gnu -dwarf-version=4 -fsyntax-only -verify %s

struct {
  char f1[100];
  int f2;
} tmp = {};

unsigned invalid1(void) { return __builtin_btf_type_id(1, tmp); } // expected-error {{__builtin_btf_type_id argument 2 not a constant}}
unsigned invalid2(void) { return __builtin_btf_type_id(1, 1, 1); } // expected-error {{too many arguments to function call, expected 2, have 3}}

int valid1(void) { return __builtin_btf_type_id(tmp, 0); }
int valid2(void) { return __builtin_btf_type_id(&tmp, 1); }
int valid3(void) { return __builtin_btf_type_id(tmp.f1[4], 10); }
