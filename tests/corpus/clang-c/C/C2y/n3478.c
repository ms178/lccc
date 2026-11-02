// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c23
// RUN: %clang_cc1 -verify -std=c2y %s
// RUN: %clang_cc1 -verify -std=c23 %s

/* WG14 N3478: Yes
 * Slay Some Earthly Demons XIII
 *
 * It was previously UB to end a source file with a partial preprocessing token
 * or a partial comment. Clang has always diagnosed these.
 */

// expected-error@+1 {{unterminated /* comment}}
/*