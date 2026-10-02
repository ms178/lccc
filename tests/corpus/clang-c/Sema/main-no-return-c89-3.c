// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c89
/* RUN: %clang_cc1 -std=c89 -fsyntax-only -verify -Wno-strict-prototypes -Wmain-return-type %s
 */

/* expected-no-diagnostics */

int main() {
    return 0;
}
