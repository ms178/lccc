// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify %s

void test_builtin_allow_sanitize_check() {
  // Test with non-string literal argument.
  char str[] = "address";
  (void)__builtin_allow_sanitize_check(str); // expected-error {{expression is not a string literal}}
  (void)__builtin_allow_sanitize_check(123); // expected-error {{expression is not a string literal}}

  // Test with unsupported sanitizer name.
  (void)__builtin_allow_sanitize_check("unsupported"); // expected-error {{invalid argument 'unsupported' to __builtin_allow_sanitize_check}}

  // Test with supported sanitizer names.
  (void)__builtin_allow_sanitize_check("address");
  (void)__builtin_allow_sanitize_check("thread");
  (void)__builtin_allow_sanitize_check("memory");
  (void)__builtin_allow_sanitize_check("hwaddress");
  (void)__builtin_allow_sanitize_check("kernel-address");
  (void)__builtin_allow_sanitize_check("kernel-memory");
  (void)__builtin_allow_sanitize_check("kernel-hwaddress");
}

#if !__has_builtin(__builtin_allow_sanitize_check)
#error "missing __builtin_allow_sanitize_check"
#endif
