// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c23: --c23: --c23
// RUN: %clang_cc1 -verify=good -std=c2y -Wall -pedantic %s
// RUN: %clang_cc1 -verify -Wnewline-eof -std=c2y -Wall -pedantic %s
// RUN: %clang_cc1 -verify=good -std=c23 -Wall -pedantic %s
// RUN: %clang_cc1 -verify=good -std=c23 %s
// RUN: %clang_cc1 -verify -Wnewline-eof -std=c23 %s

/* WG14 N3411: Yes
 * Slay Some Earthly Demons XII
 *
 * Allow a non-empty source file to end without a final newline character. Note
 * that this file intentionally does not end with a trailing newline.
 */
// good-no-diagnostics

int x; // Ensure the file contains at least one declaration.
// expected-warning {{no newline at end of file}}