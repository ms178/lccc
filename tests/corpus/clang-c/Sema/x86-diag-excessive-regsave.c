// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  -DNO_CALLER_SAVED_REGISTERS=1 -DGPRONLY=1 --c
// RUN: %clang_cc1 -triple x86_64-unknown-linux-gnu -fsyntax-only -verify %s
// RUN: %clang_cc1 -triple x86_64-unknown-linux-gnu -fsyntax-only -verify %s -DNO_CALLER_SAVED_REGISTERS=1
// RUN: %clang_cc1 -triple x86_64-unknown-linux-gnu -target-feature -x87 -target-feature -mmx -target-feature -sse -fsyntax-only -verify %s -DGPRONLY=1

#if defined(NO_CALLER_SAVED_REGS) || defined(GPRONLY)
// expected-no-diagnostics
#else
#define EXPECT_WARNING
#endif

#ifdef NO_CALLER_SAVED_REGS
__attribute__((no_caller_saved_registers))
#endif
#ifdef EXPECT_WARNING
// expected-note@+3 {{'foo' declared here}}
// expected-note@+2 {{'foo' declared here}}
#endif
extern void foo(void *);

__attribute__((no_caller_saved_registers))
void no_caller_saved_registers(void *arg) {
#ifdef EXPECT_WARNING
// expected-warning@+2 {{function with attribute 'no_caller_saved_registers' should only call a function with attribute 'no_caller_saved_registers' or be compiled with '-mgeneral-regs-only'}}
#endif
    foo(arg);
}

__attribute__((interrupt))
void interrupt(void *arg) {
#ifdef EXPECT_WARNING
// expected-warning@+2 {{interrupt service routine should only call a function with attribute 'no_caller_saved_registers' or be compiled with '-mgeneral-regs-only'}}
#endif
    foo(arg);
}
