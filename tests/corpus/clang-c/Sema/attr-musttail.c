// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -verify -fsyntax-only %s

int NotAProtoType(); // expected-note{{add 'void' to the parameter list to turn an old-style K&R function declaration into a prototype}}
int TestCalleeNotProtoType(void) {
  __attribute__((musttail)) return NotAProtoType(); // expected-error{{'musttail' attribute requires that both caller and callee functions have a prototype}}
}

int ProtoType(void);
int TestCallerNotProtoType() {                  // expected-note{{add 'void' to the parameter list to turn an old-style K&R function declaration into a prototype}}
  __attribute__((musttail)) return ProtoType(); // expected-error{{'musttail' attribute requires that both caller and callee functions have a prototype}}
}

int TestProtoType(void) {
  return ProtoType();
}
