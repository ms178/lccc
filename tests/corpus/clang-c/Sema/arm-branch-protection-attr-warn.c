// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -triple thumbv6m -verify -fsyntax-only %s

// expected-warning@+1 {{unsupported 'branch-protection' in the 'target' attribute string; 'target' attribute ignored}}
__attribute__((target("arch=cortex-m0,branch-protection=bti"))) void f1(void) {}

// expected-warning@+1 {{unsupported 'branch-protection' in the 'target' attribute string; 'target' attribute ignored}}
__attribute__((target("arch=cortex-m0,branch-protection=pac-ret"))) void f2(void) {}

// expected-warning@+1 {{unsupported 'branch-protection' in the 'target' attribute string; 'target' attribute ignored}}
__attribute__((target("arch=cortex-m0,branch-protection=bti+pac-ret"))) void f3(void) {}

// expected-warning@+1 {{unsupported 'branch-protection' in the 'target' attribute string; 'target' attribute ignored}}
__attribute__((target("arch=cortex-m0,branch-protection=bti+pac-ret+leaf"))) void f4(void) {}

// expected-warning@+1 {{unsupported 'branch-protection' in the 'target' attribute string; 'target' attribute ignored}}
__attribute__((target("arch=cortex-a17,thumb,branch-protection=bti+pac-ret+leaf"))) void f5(void) {}
