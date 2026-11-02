// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c23
// RUN: %clang_cc1 -std=c23 %s -fsyntax-only -verify
// REQUIRES: system-linux


int urandom[] = {
#embed "/dev/urandom" //expected-error {{device files are not yet supported by '#embed' directive}}
};
int random[] = {
#embed "/dev/random" //expected-error {{device files are not yet supported by '#embed' directive}}
};
int zero[] = {
#embed "/dev/zero" //expected-error {{device files are not yet supported by '#embed' directive}}
};
int null[] = {
#embed "/dev/null" //expected-error {{device files are not yet supported by '#embed' directive}}
};
