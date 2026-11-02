// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -emit-llvm -o - %s

/* WG14 N1311: Yes
 * Initializing static or external variables
 */

static int x;
static union {
  void *vp;
  float f;
  int i;
} u;

int main(void) {
  return x + u.i;
}

// CHECK: @x ={{.*}}i32 0
// CHECK-NEXT: @u ={{.*}}zeroinitializer
