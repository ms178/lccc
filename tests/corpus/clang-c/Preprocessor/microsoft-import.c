// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --ms_compatibility --c
// RUN: %clang_cc1 -E -verify -fms-compatibility %s

#import "pp-record.h" // expected-error {{#import of type library is an unsupported Microsoft feature}}

// Test attributes
#import "pp-record.h" no_namespace, auto_rename // expected-error {{#import of type library is an unsupported Microsoft feature}}

#import "pp-record.h" no_namespace \
                      auto_rename \
                      auto_search
// expected-error@-3 {{#import of type library is an unsupported Microsoft feature}}

