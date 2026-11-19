#!/usr/bin/env python3
"""An asm goto must behave like the `if (c) goto label;` it stands for.

`asm goto("test; jnz %l[out]" : : "r"(c) : "cc" : out)` and
`if (c) goto out;` are the same control flow.  The compiler sees the second
as a branch that ends its block.  It sees the first as an instruction, and an
instruction that can leave the function's straight line is a thing each part
of the compiler has to know about on its own: where a block may be left, what
has run on the way out, which labels a jump can reach.  A part that does not
know keeps a stale value, drops a store or reads a register nobody wrote --
silently, at one optimization level, for one storage class.

So this gate does not ask each part.  It writes every function twice, once
with the asm goto and once with the plain branch, over a table of shapes
(what stands ahead of the jump, what stands behind it, where the label is,
which loop is around it) times a table of storage kinds (a register local, an
escaped local, a global, memory behind a pointer, an array element, a 64-bit
value, a double, a byte, a volatile).  `main` runs both on the same inputs and
compares the return value and every byte of state they could have written.
The plain branch is the reference: no other compiler is needed to tell a
wrong answer, and none of the expected values is written down by hand.

    check_asm_goto_equivalence.py --lccc PATH        build and run the matrix
    check_asm_goto_equivalence.py --emit             print the C source
    check_asm_goto_equivalence.py --write            refresh the committed fixture
    check_asm_goto_equivalence.py --check-committed  the fixture is what --emit prints
    check_asm_goto_equivalence.py --self-test        the gate sees a planted fault
    check_asm_goto_equivalence.py --reference CC     run the matrix on another compiler
    check_asm_goto_equivalence.py --lccc PATH --cross
                                                     and on AArch64 and RISC-V (qemu-user)
    check_asm_goto_equivalence.py --lccc PATH --cross-only
                                                     those two alone
    check_asm_goto_equivalence.py --lccc PATH --cross-only --self-test-only
                                                     their planted faults, no rows

The committed copy, `asm_goto_equivalence.c`, runs with the regression corpus
at -O2; this gate adds the other optimization levels, -m32 on the host, and
with --cross the two other targets at every level.  Each program also checks
that the registers a function has to preserve came back unchanged.

check_switch_cross_execute.py runs the committed copy on the two other
targets at every level as well, with `printf` a no-op: it knows that a pair
differs and not which.  The --cross rows are the same programs with the
answer printed, and --cross-only --self-test-only is the part nothing else
has: a register lost on purpose on each target, to see that the check which
travels in the fixture reports it there.
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
FIXTURE = HERE / "asm_goto_equivalence.c"

# ── storage kinds ───────────────────────────────────────────────────────────
# name -> (declaration at function entry, read expression, write statement
#          taking an int expression, result expression)
KINDS = {
    "reg": ("int v = c->a;", "v", "v = ({e});", "v"),
    "esc": ("int v = c->a; sink(&v);", "v", "v = ({e});", "v"),
    "glob": ("", "G", "G = ({e});", "G"),
    "ptr": ("", "c->v", "c->v = ({e});", "c->v"),
    "elem": ("", "c->arr[n & 3]", "c->arr[n & 3] = ({e});", "c->arr[n & 3]"),
    "wide": (
        "long long v = c->w;",
        "(int)(v >> 32)",
        "v = (long long)({e}) * 0x100000003LL;",
        "(int)(v >> 32) + (int)(v & 0xffff)",
    ),
    "fp": ("double v = c->d;", "(int)v", "v = ({e}) + 0.25;", "(int)(v * 4)"),
    "byte": (
        "unsigned char v = (unsigned char)c->a;",
        "v",
        "v = (unsigned char)({e});",
        "v",
    ),
    "vol": ("volatile int v = c->a;", "v", "v = ({e});", "v"),
    "agg": (
        "struct pair v = { c->a, c->v };",
        "(v.lo + v.hi)",
        "v = mk_pair({e});",
        "v.lo * 3 + v.hi",
    ),
    "bits": (
        "struct bits v = { 3, (unsigned)c->a };",
        "(int)v.s",
        "v.s = (unsigned)({e});",
        "(int)(v.f + v.s)",
    ),
    "arr": (
        "int v[4] = { c->a, 1, 2, 3 };",
        "v[n & 3]",
        "v[n & 3] = ({e});",
        "v[0] + 3 * v[1] + 5 * v[2] + 7 * v[3]",
    ),
}

# ── shapes ──────────────────────────────────────────────────────────────────
# Bodies are written with WR(e) for a write, RD for a read, RET for the
# result and BR(c, label) for the jump.  Every loop ends for every input:
# x is in [-1, 2], y in [0, 1], n in {0, 1, 3, 4}.
SHAPES = {
    # the label joins the fallthrough
    "join": "WR(1); BR(x, out); WR(2); out: return RET;",
    "join_call": "WR(1); BR(x, out); WR(g(n)); out: return RET;",
    "untouched": "BR(x, out); WR(3); WR(4); out: return RET;",
    "rmw_behind": "WR(5); BR(x, out); WR(RD + n); WR(RD * 3); out: return RET;",
    "call_between": "WR(1); g(0); BR(x, out); g(1); WR(2); g(2); out: return RET;",
    "call_ahead": "int t = g(n); WR(t); BR(x, out); WR(t + 1); out: return RET + t;",
    "ternary_cond": "WR(1); BR(x ? y : n, out); WR(2); out: return RET;",
    # the label has a tail of its own
    "overwritten": "WR(1); BR(x, out); WR(2); return RET + 100; out: return RET;",
    "label_writes": "WR(1); BR(x, out); WR(2); return RET; out: { int r = RET; WR(9); return r; }",
    # the label's block holds a call the inliner expands in its second run,
    # when nothing but the asm goto names the label any more
    "label_inlines": (
        "WR(1); BR(x, out); WR(2); return RET;"
        " out: WR(RD + 10); WR(RD + pick(&c->a)); return RET;"
    ),
    "loop_label_inlines": (
        "int i; WR(0); for (i = 0; i < n; i++) { BR(i == x, out); WR(RD + 10 + i); }"
        " return RET; out: WR(RD + 3); WR(RD + pick(&c->a) + i); return RET;"
    ),
    # more than one way out
    "two_same": "WR(1); BR(x, out); WR(2); BR(y, out); WR(3); out: return RET;",
    "two_labels": (
        "WR(1); BR(x, la); WR(2); BR(y, lb); WR(3); return RET;"
        " la: return RET + 10; lb: return RET + 20;"
    ),
    "cond_compare": "WR(1); BR(x < y, out); WR(2); BR(n == 3, out); WR(3); out: return RET;",
    "label_chain": (
        "WR(1); BR(x, out); WR(2); out: WR(RD + 5); BR(y, done); WR(RD + 7);"
        " done: return RET;"
    ),
    # branches around it
    "nested_if": "if (n) { WR(1); BR(x, out); WR(2); } else { WR(5); } out: return RET;",
    "both_arms": (
        "if (n) { WR(1); BR(x, out); WR(2); } else { WR(5); BR(y, out); WR(6); }"
        " WR(RD + 10); out: return RET;"
    ),
    "switch_arm": (
        "switch (n & 3) { case 0: WR(1); BR(x, out); WR(2); break;"
        " case 1: WR(3); BR(y, out); /* fall through */ case 2: WR(RD + 4); break;"
        " default: WR(9); } WR(RD + 100); out: return RET;"
    ),
    # a loop left by it
    "counted_exit": (
        "int i; WR(7); for (i = 0; i < n; i++) { int t = g(i); BR(i == x, out); WR(t); }"
        " out: return RET;"
    ),
    "endless_exit": (
        "int i = 0, stop = x < 0 ? 0 : x; WR(7);"
        " for (;;) { int t = g(i); BR(i == stop, out); WR(t); i++; } out: return RET;"
    ),
    "accumulate": (
        "int i; WR(0); for (i = 0; i < n; i++) { BR(i == x, out); WR(RD + 10 + i); }"
        " out: return RET;"
    ),
    "do_while": (
        "int j = 0; WR(0); do { BR(j == x, out); WR(RD + 10 + j); } while (++j < n);"
        " out: return RET;"
    ),
    "exit_inner": (
        "int i, j; WR(0); for (i = 0; i < n; i++) for (j = 0; j < 3; j++)"
        " { BR(i * 3 + j == x + 4, out); WR(RD + 1); } out: return RET;"
    ),
    "skip_loop": (
        "int i; WR(1); BR(x, out); for (i = 0; i < n; i++) WR(RD + i); out: return RET;"
    ),
    # a loop made by it
    "backward": "int i = 0; WR(0); again: WR(RD + i); i++; BR(i < n, again); return RET;",
    "backward_two": (
        "int i = 0; WR(0); again: i++; BR(i < x, again); WR(RD + i); BR(i < n, again);"
        " return RET;"
    ),
    # the asm goto in an inlined function: a static key, as the kernel has them
    "inl_if": "WR(1); if (KEY(x)) { WR(2); } else { WR(3); } return RET;",
    "inl_or": "WR(1); if (KEY(x) || KEY(y)) WR(RD + 2); WR(RD * 2); return RET;",
    "inl_early": "WR(1); if (KEY(x)) return RET + 50; WR(2); return RET;",
    "inl_loop": (
        "int i; WR(0); for (i = 0; i < n; i++) { if (KEY(i == x)) break; WR(RD + 10 + i); }"
        " return RET;"
    ),
    # an asm goto with an output, read where every compiler defines it: on the
    # way that falls through
    "out_value": (
        "int t = 3; WR(1); OUT(x, t, n + 5, out); WR(RD + t); return RET; out: return RET + 100;"
    ),
    "out_loop": (
        "int i, t = 0; WR(0); for (i = 0; i < n; i++) { OUT(i == x, t, i + 20, out); WR(RD + t); }"
        " out: return RET;"
    ),
    # a loop entered by it, past its head
    "into_loop": (
        "int i = 0; WR(1); BR(x, mid); for (i = 0; i < n; i++) { WR(RD + 1); mid: WR(RD + 2); }"
        " return RET;"
    ),
}

# Shapes with two values of their own; one copy each, not one per kind.
EXTRA = {
    "swap_carried": (
        "int a2 = c->a, b2 = c->v, i; for (i = 0; i < n; i++)"
        " { int t = g(i); BR(i == x, out); { int old = a2; a2 = b2 + t; b2 = old; } }"
        " out: return a2 * 1000 + b2;"
    ),
    "pointer_walk": (
        "const int *p = c->arr, *last = 0; int k = 0;"
        " for (;;) { int e = *p + zero; BR(e == 13 + x || k == 3, out); last = p; p++; k++; }"
        " out: return last ? *last + k : -1;"
    ),
    "three_live": (
        "int a2 = g(x), b2 = g(y), c2 = g(n); BR(x, out); a2 = b2 + 1; b2 = c2 + 2; c2 = 3;"
        " out: return a2 + 3 * b2 + 7 * c2;"
    ),
    "mixed_storage": (
        "int v = c->a; G = 1; c->v = 2; BR(x, out); v = 3; G = 4; c->v = 5;"
        " BR(y, out); v = 6; G = 7; out: return v * 100 + G * 10 + c->v;"
    ),
}

HEADER = """\
/* An asm goto and the plain branch it stands for, side by side.
 *
 * GENERATED by tests/regression/check_asm_goto_equivalence.py --write.
 * Do not edit: change the tables in that script and write it again
 * (`--check-committed` keeps the two in step).
 *
 * Each shape is compiled twice: `NAME_asm` jumps with
 * `asm goto("...jnz %l[out]" : : "r"(c) : "cc" : out)`, `NAME_ref` with
 * `if (c) goto out;`.  They are the same program, so `main` runs both on the
 * same inputs and compares the return value and all the state either could
 * have written.  A difference means some part of the compiler took the asm
 * goto for a line that cannot leave its block: it kept a stale value, dropped
 * a store behind the jump, or forgot the label can be reached from there.
 *
 * Found with it on main (GCC 16.2 and Clang 23.1.5 are right): a store to a
 * global or an escaped local ahead of the asm goto deleted as dead (-O2), a
 * stack slot forwarded across the label of an asm goto (x86-64, -O1), a
 * register holding a loop-carried value given up as dead when only the asm
 * goto leaves the loop (-m32, -O1), and a label moved behind the call inlined
 * into its block, so that the asm goto skipped the call (-O2; the fault paths
 * of the kernel's copy_from_kernel_nofault()).
 *
 * Header-free: it is built for -m32 and for the cross targets as well.
 */
extern int printf(const char *, ...);

#if (defined(__x86_64__) || defined(__i386__)) && defined(ASM_GOTO_RAW_JUMP)
/* The same jump spelled as bytes, the way a patched static key or an
 * alternative reaches its label: no line of the text says `jnz`.  A part of
 * the compiler that finds jumps by their mnemonic sees a label nobody goes to. */
#define ASM_BR(c, label)                                                                 \\
    __asm__ goto("testl %0, %0\\n\\t.byte 0x0f, 0x85\\n\\t.long %l[" #label "] - . - 4" \\
                 : : "r"((int)(c)) : "cc" : label)
#elif defined(__x86_64__) || defined(__i386__)
#define ASM_BR(c, label) \\
    __asm__ goto("testl %0, %0; jnz %l[" #label "]" : : "r"((int)(c)) : "cc" : label)
#elif defined(__aarch64__)
#define ASM_BR(c, label) __asm__ goto("cbnz %w0, %l[" #label "]" : : "r"((int)(c)) : : label)
#elif defined(__riscv)
#define ASM_BR(c, label) __asm__ goto("bnez %0, %l[" #label "]" : : "r"((int)(c)) : : label)
#else
#error unsupported target
#endif
#define REF_BR(c, label)   \\
    do {                   \\
        if ((int)(c))      \\
            goto label;    \\
    } while (0)

/* The same with an output: `dst = src`, then the jump. */
#if (defined(__x86_64__) || defined(__i386__)) && defined(ASM_GOTO_RAW_JUMP)
#define ASM_OUT(c, dst, src, label)                                                      \\
    __asm__ goto("movl %2, %0\\n\\ttestl %1, %1\\n\\t.byte 0x0f, 0x85\\n\\t.long %l[" #label \\
                 "] - . - 4"                                                             \\
                 : "=&r"(dst) : "r"((int)(c)), "r"((int)(src)) : "cc" : label)
#elif defined(__x86_64__) || defined(__i386__)
#define ASM_OUT(c, dst, src, label)                                    \\
    __asm__ goto("movl %2, %0; testl %1, %1; jnz %l[" #label "]"       \\
                 : "=&r"(dst) : "r"((int)(c)), "r"((int)(src)) : "cc" : label)
#elif defined(__aarch64__)
#define ASM_OUT(c, dst, src, label)                                    \\
    __asm__ goto("mov %w0, %w2\\n\\tcbnz %w1, %l[" #label "]"          \\
                 : "=&r"(dst) : "r"((int)(c)), "r"((int)(src)) : : label)
#else
#define ASM_OUT(c, dst, src, label)                                    \\
    __asm__ goto("mv %0, %2\\n\\tbnez %1, %l[" #label "]"              \\
                 : "=&r"(dst) : "r"((int)(c)), "r"((int)(src)) : : label)
#endif
#define REF_OUT(c, dst, src, label) \\
    do {                            \\
        (dst) = (int)(src);         \\
        if ((int)(c))               \\
            goto label;             \\
    } while (0)

#define NOINLINE __attribute__((noinline))

struct cell {
    int a;
    int v;
    int arr[4];
    long long w;
    double d;
};

struct pair {
    int lo, hi;
};

struct bits {
    unsigned f : 5;
    unsigned s : 11;
};

static inline struct pair mk_pair(int e)
{
    struct pair p = { e, e + 1 };
    return p;
}

static volatile int zero;
int G;

NOINLINE static void sink(void *p)
{
    (void)p;
    zero = zero;
}

NOINLINE static int g(int i)
{
    return 100 + i + zero;
}

/* `*p`, written the way the kernel's get_current() and this_cpu_read() are: a
 * switch over a constant size.  It is two dozen blocks and a handful of
 * instructions until the first cleanup folds the switch, which is too many
 * blocks for the inliner's first run; the second run takes it, after the
 * unreachable code ahead of a label has been swept. */
NOINLINE static int pick_bad_size(void)
{
    return -1 + zero;
}

static inline __attribute__((always_inline)) int pick(const int *p)
{
    int value;
    switch (sizeof(*p)) {
    case 1: value = *(const signed char *)p; break;
    case 2: value = *(const short *)p; break;
    case 4: value = *p; break;
    case 8: value = (int)*(const long long *)p; break;
    default: value = pick_bad_size(); break;
    }
    switch (sizeof(value)) {
    case 1: value += 1; break;
    case 2: value += 2; break;
    case 4: value += zero; break;
    case 8: value += 8; break;
    default: value = pick_bad_size(); break;
    }
    return value;
}

/* A static key: the asm goto lives in a function that is always inlined. */
static inline __attribute__((always_inline)) int key_asm(int c)
{
    ASM_BR(c, yes);
    return 0;
yes:
    return 1;
}

static inline __attribute__((always_inline)) int key_ref(int c)
{
    REF_BR(c, yes);
    return 0;
yes:
    return 1;
}

/* Call fn(x, y, n, c) with known values in the registers a function has to
 * preserve, store what it returns, and answer with a mask of the registers
 * that came back changed.  A function that loses one of them is not found by
 * comparing results: its caller just goes on with a wrong loop counter. */
typedef int shape_fn(int, int, int, struct cell *);
#if defined(__i386__)
int guarded_call(shape_fn *fn, int x, int y, int n, struct cell *c, int *result)
    __attribute__((visibility("hidden")));
__asm__(".text\\n"
        ".globl guarded_call\\n"
        ".hidden guarded_call\\n"
        ".type guarded_call, @function\\n"
        "guarded_call:\\n"
        "    pushl %ebp\\n"
        "    pushl %ebx\\n"
        "    pushl %esi\\n"
        "    pushl %edi\\n"
        "    subl $12, %esp\\n"
        "    pushl 48(%esp)\\n" /* c */
        "    pushl 48(%esp)\\n" /* n */
        "    pushl 48(%esp)\\n" /* y */
        "    pushl 48(%esp)\\n" /* x */
        "    movl $0x1b1b1b1b, %ebx\\n"
        "    movl $0x2c2c2c2c, %esi\\n"
        "    movl $0x3d3d3d3d, %edi\\n"
        "    movl $0x4e4e4e4e, %ebp\\n"
        "    call *48(%esp)\\n"
        "    addl $28, %esp\\n"
        "    movl 40(%esp), %ecx\\n"
        "    movl %eax, (%ecx)\\n"
        "    xorl %eax, %eax\\n"
        "    cmpl $0x1b1b1b1b, %ebx\\n"
        "    je 1f\\n"
        "    orl $1, %eax\\n"
        "1:  cmpl $0x2c2c2c2c, %esi\\n"
        "    je 1f\\n"
        "    orl $2, %eax\\n"
        "1:  cmpl $0x3d3d3d3d, %edi\\n"
        "    je 1f\\n"
        "    orl $4, %eax\\n"
        "1:  cmpl $0x4e4e4e4e, %ebp\\n"
        "    je 1f\\n"
        "    orl $8, %eax\\n"
        "1:  popl %edi\\n"
        "    popl %esi\\n"
        "    popl %ebx\\n"
        "    popl %ebp\\n"
        "    ret\\n"
        ".size guarded_call, .-guarded_call\\n");
#elif defined(__x86_64__)
int guarded_call(shape_fn *fn, int x, int y, int n, struct cell *c, int *result)
    __attribute__((visibility("hidden")));
__asm__(".text\\n"
        ".globl guarded_call\\n"
        ".hidden guarded_call\\n"
        ".type guarded_call, @function\\n"
        "guarded_call:\\n"
        "    pushq %rbp\\n"
        "    pushq %rbx\\n"
        "    pushq %r12\\n"
        "    pushq %r13\\n"
        "    pushq %r14\\n"
        "    pushq %r15\\n"
        "    pushq %r9\\n" /* result; the stack is aligned for the call again */
        "    movq %rdi, %rax\\n"
        "    movl %esi, %edi\\n"
        "    movl %edx, %esi\\n"
        "    movl %ecx, %edx\\n"
        "    movq %r8, %rcx\\n"
        "    movq $0x1b1b1b1b, %rbx\\n"
        "    movq $0x2c2c2c2c, %rbp\\n"
        "    movq $0x3d3d3d3d, %r12\\n"
        "    movq $0x4e4e4e4e, %r13\\n"
        "    movq $0x5f5f5f5f, %r14\\n"
        "    movq $0x6a6a6a6a, %r15\\n"
        "    call *%rax\\n"
        "    popq %r9\\n"
        "    movl %eax, (%r9)\\n"
        "    xorl %eax, %eax\\n"
        "    cmpq $0x1b1b1b1b, %rbx\\n"
        "    je 1f\\n"
        "    orl $1, %eax\\n"
        "1:  cmpq $0x2c2c2c2c, %rbp\\n"
        "    je 1f\\n"
        "    orl $2, %eax\\n"
        "1:  cmpq $0x3d3d3d3d, %r12\\n"
        "    je 1f\\n"
        "    orl $4, %eax\\n"
        "1:  cmpq $0x4e4e4e4e, %r13\\n"
        "    je 1f\\n"
        "    orl $8, %eax\\n"
        "1:  cmpq $0x5f5f5f5f, %r14\\n"
        "    je 1f\\n"
        "    orl $16, %eax\\n"
        "1:  cmpq $0x6a6a6a6a, %r15\\n"
        "    je 1f\\n"
        "    orl $32, %eax\\n"
        "1:  popq %r15\\n"
        "    popq %r14\\n"
        "    popq %r13\\n"
        "    popq %r12\\n"
        "    popq %rbx\\n"
        "    popq %rbp\\n"
        "    ret\\n"
        ".size guarded_call, .-guarded_call\\n");
"""

# The same guard for the targets whose callee-saved set is long enough to be
# worth a table: (register, the value it holds across the call).  The bit a
# lost register sets in the answer is its position in GPR followed by FPR.
A64_GPR = [(f"x{19 + i}", 0x1B1 + 0x111 * i) for i in range(10)]
A64_FPR = [(f"d{8 + i}", 0x111 * (i + 1)) for i in range(8)]
RV64_GPR = [(f"s{i}", 0x1B1 + 0x111 * i) for i in range(12)]
RV64_FPR = [(f"fs{i}", 0x111 * (i + 1)) for i in range(12)]


def _file_scope_asm(lines: list[str]) -> str:
    """`__asm__("...\\n" ...);` at file scope, one C string per line."""
    quoted = "\n".join(f'        "{line}\\n"' for line in lines)
    return '__asm__(".text\\n"\n' + quoted + ");\n"


def guard_aarch64() -> str:
    """AAPCS64: x19-x28 and the low halves of v8-v15 belong to the caller."""
    frame = 16 + 8 * (len(A64_GPR) + len(A64_FPR)) + 16
    result_at = frame - 16
    pairs_of = lambda regs: [(regs[i][0], regs[i + 1][0]) for i in range(0, len(regs), 2)]
    saved = pairs_of(A64_GPR) + pairs_of(A64_FPR)
    lines = [
        ".globl guarded_call",
        ".hidden guarded_call",
        ".type guarded_call, %function",
        "guarded_call:",
        f"    stp x29, x30, [sp, #-{frame}]!",
    ]
    lines += [f"    stp {a}, {b}, [sp, #{16 + 16 * i}]" for i, (a, b) in enumerate(saved)]
    lines += [
        f"    str x5, [sp, #{result_at}]",  # result
        "    mov x9, x0",
        "    mov w0, w1",
        "    mov w1, w2",
        "    mov w2, w3",
        "    mov x3, x4",
    ]
    lines += [f"    mov {reg}, #{value:#x}" for reg, value in A64_GPR]
    for reg, value in A64_FPR:
        lines += [f"    mov x10, #{value:#x}", f"    fmov {reg}, x10"]
    lines += [
        "    blr x9",
        f"    ldr x5, [sp, #{result_at}]",
        "    str w0, [x5]",
        "    mov w0, #0",
    ]
    label = "    "
    for bit, (reg, value) in enumerate(A64_GPR + A64_FPR):
        if reg.startswith("d"):
            lines.append(f"{label}fmov x10, {reg}")
            label, reg = "    ", "x10"
        lines += [
            f"{label}cmp {reg}, #{value:#x}",
            "    b.eq 1f",
            f"    orr w0, w0, #{1 << bit:#x}",
        ]
        label = "1:  "
    lines += [
        f"{label}ldp {a}, {b}, [sp, #{16 + 16 * i}]"
        if i == len(saved) - 1
        else f"    ldp {a}, {b}, [sp, #{16 + 16 * i}]"
        for i, (a, b) in reversed(list(enumerate(saved)))
    ]
    lines += [
        f"    ldp x29, x30, [sp], #{frame}",
        "    ret",
        ".size guarded_call, .-guarded_call",
    ]
    return _file_scope_asm(lines)


def guard_riscv64() -> str:
    """LP64D: s0-s11 and fs0-fs11 belong to the caller."""
    slots = ["ra"] + [reg for reg, _ in RV64_GPR + RV64_FPR]
    frame = (8 * (len(slots) + 1) + 15) // 16 * 16
    result_at = 8 * len(slots)
    store = lambda reg: "fsd" if reg.startswith("f") else "sd"
    load = lambda reg: "fld" if reg.startswith("f") else "ld"
    lines = [
        ".globl guarded_call",
        ".hidden guarded_call",
        ".type guarded_call, @function",
        "guarded_call:",
        f"    addi sp, sp, -{frame}",
    ]
    lines += [f"    {store(reg)} {reg}, {8 * i}(sp)" for i, reg in enumerate(slots)]
    lines += [
        f"    sd a5, {result_at}(sp)",  # result
        "    mv t0, a0",
        "    mv a0, a1",
        "    mv a1, a2",
        "    mv a2, a3",
        "    mv a3, a4",
    ]
    lines += [f"    li {reg}, {value:#x}" for reg, value in RV64_GPR]
    for reg, value in RV64_FPR:
        lines += [f"    li t1, {value:#x}", f"    fmv.d.x {reg}, t1"]
    lines += [
        "    jalr t0",
        f"    ld a5, {result_at}(sp)",
        "    sw a0, 0(a5)",
        "    li a0, 0",
    ]
    label = "    "
    for bit, (reg, value) in enumerate(RV64_GPR + RV64_FPR):
        if reg.startswith("f"):
            lines.append(f"{label}fmv.x.d t2, {reg}")
            label, reg = "    ", "t2"
        lines += [f"{label}li t1, {value:#x}", f"    beq {reg}, t1, 1f"]
        if 1 << bit < 2048:
            lines.append(f"    ori a0, a0, {1 << bit}")
        else:
            lines += [f"    li t2, {1 << bit:#x}", "    or a0, a0, t2"]
        label = "1:  "
    lines.append(f"{label}{load(slots[0])} {slots[0]}, 0(sp)")
    lines += [f"    {load(reg)} {reg}, {8 * i}(sp)" for i, reg in enumerate(slots) if i]
    lines += [
        f"    addi sp, sp, {frame}",
        "    ret",
        ".size guarded_call, .-guarded_call",
    ]
    return _file_scope_asm(lines)


GUARD_PROTOTYPE = (
    "int guarded_call(shape_fn *fn, int x, int y, int n, struct cell *c, int *result)\n"
    '    __attribute__((visibility("hidden")));\n'
)
HEADER += (
    "#elif defined(__aarch64__)\n"
    + GUARD_PROTOTYPE
    + guard_aarch64()
    + "#elif defined(__riscv) && __riscv_xlen == 64 && defined(__riscv_float_abi_double)\n"
    + GUARD_PROTOTYPE
    + guard_riscv64()
    + """\
#else
static int guarded_call(shape_fn *fn, int x, int y, int n, struct cell *c, int *result)
{
    *result = fn(x, y, n, c);
    return 0;
}
#endif
"""
)

FOOTER = """\

static void reset(struct cell *c)
{
    c->a = 50;
    c->v = 60;
    c->arr[0] = 11;
    c->arr[1] = 12;
    c->arr[2] = 13;
    c->arr[3] = 14;
    c->w = 50 * 0x100000003LL;
    c->d = 50.25;
    G = 70;
}

static int same_state(const struct cell *p, const struct cell *q)
{
    return p->a == q->a && p->v == q->v && p->arr[0] == q->arr[0] && p->arr[1] == q->arr[1] &&
           p->arr[2] == q->arr[2] && p->arr[3] == q->arr[3] && p->w == q->w && p->d == q->d;
}

int main(void)
{
    static const int xs[] = { -1, 0, 1, 2 }, ys[] = { 0, 1 }, ns[] = { 0, 1, 3, 4 };
    unsigned i, xi, yi, ni;
    int bad = 0;

    for (i = 0; i < sizeof pairs / sizeof pairs[0]; i++) {
        int reported = 0;
        for (xi = 0; xi < 4; xi++)
            for (yi = 0; yi < 2; yi++)
                for (ni = 0; ni < 4; ni++) {
                    int x = xs[xi] + zero, y = ys[yi] + zero, n = ns[ni] + zero;
                    struct cell ca, cr;
                    int ra, rr, ga, gr, lost_a, lost_r;
                    reset(&ca);
                    lost_a = guarded_call(pairs[i].with_asm, x, y, n, &ca, &ra);
                    ga = G;
                    reset(&cr);
                    lost_r = guarded_call(pairs[i].with_branch, x, y, n, &cr, &rr);
                    gr = G;
                    if ((lost_a | lost_r) && !reported)
                        printf("MISMATCH %s x=%d y=%d n=%d: callee-saved registers lost, "
                               "mask %#x with the asm goto, %#x with the branch\\n",
                               pairs[i].name, x, y, n, lost_a, lost_r);
                    else if (ra == rr && ga == gr && same_state(&ca, &cr))
                        continue;
                    else if (!reported)
                        printf("MISMATCH %s x=%d y=%d n=%d: asm goto %d (G %d), branch %d (G %d)%s\\n",
                               pairs[i].name, x, y, n, ra, ga, rr, gr,
                               same_state(&ca, &cr) ? "" : ", memory differs");
                    reported = 1;
                }
        bad += reported;
    }
    if (bad) {
        printf("asm goto equivalence: %d of %u shapes differ\\n", bad,
               (unsigned)(sizeof pairs / sizeof pairs[0]));
        return 1;
    }
    printf("asm goto equivalence: %u shapes, 32 inputs each: ok\\n",
           (unsigned)(sizeof pairs / sizeof pairs[0]));
    return 0;
}
"""


def jumps(body: str, flavour: str) -> str:
    """Pick the way to jump: `flavour` is "asm" or "ref"."""
    return (
        body.replace("KEY(", f"key_{flavour}(")
        .replace("OUT(", f"{flavour.upper()}_OUT(")
        .replace("BR(", f"{flavour.upper()}_BR(")
    )


def expand(body: str, kind: str, flavour: str) -> str:
    """Spell out one body for one storage kind and one way to jump."""
    _, read, write, result = KINDS[kind]
    out = []
    i = 0
    while i < len(body):
        if body.startswith("WR(", i):
            depth, j = 1, i + 3
            while depth:
                depth += {"(": 1, ")": -1}.get(body[j], 0)
                j += 1
            inner = body[i + 3 : j - 1].replace("RD", read)
            out.append(write.replace("{e}", inner))
            i = j
            if i < len(body) and body[i] == ";":
                i += 1
            continue
        if body.startswith("BR(", i):
            out.append(f"{flavour.upper()}_BR(")
            i += 3
            continue
        if body.startswith("KEY(", i):
            out.append(f"key_{flavour}(")
            i += 4
            continue
        if body.startswith("OUT(", i):
            out.append(f"{flavour.upper()}_OUT(")
            i += 4
            continue
        if body.startswith("RET", i):
            out.append("(" + result + ")")
            i += 3
            continue
        if body.startswith("RD", i):
            out.append("(" + read + ")")
            i += 2
            continue
        out.append(body[i])
        i += 1
    return "".join(out)


def function(name: str, decl: str, body: str, sabotage: str | None = None) -> str:
    # The planted faults of --self-test.
    if sabotage == "value":
        # the asm goto copy writes another value behind the jump
        assert "(2)" in body, name
        body = body.replace("(2)", "(2 + 1)", 1)
    elif sabotage in LOSES_A_REGISTER:
        # the asm goto copy loses a register its caller owns
        body = f'__asm__("{LOSES_A_REGISTER[sabotage][0]}"); ' + body
    lines = [f"NOINLINE static int {name}(int x, int y, int n, struct cell *c)", "{"]
    lines.append("    (void)x; (void)y; (void)n; (void)c;")
    if decl:
        lines.append("    " + decl)
    # one statement per line keeps a debugger and a diff readable
    depth = 1
    stmt = ""
    for ch in body:
        stmt += ch
        if ch == "{":
            lines.append("    " * depth + stmt.strip())
            stmt = ""
            depth += 1
        elif ch == "}":
            depth -= 1
            head = stmt[:-1].strip()
            if head:
                lines.append("    " * (depth + 1) + head)
            lines.append("    " * depth + "}")
            stmt = ""
        elif ch == ";" and stmt.count("(") == stmt.count(")"):
            lines.append("    " * depth + stmt.strip())
            stmt = ""
    if stmt.strip():
        lines.append("    " * depth + stmt.strip())
    lines.append("}")
    return "\n".join(lines)


def pairs() -> list[tuple[str, str, str]]:
    """(name, declaration, body with RD/WR/BR/RET still in it) for every pair."""
    table = []
    for shape, body in SHAPES.items():
        for kind in KINDS:
            table.append((f"{shape}_{kind}", kind, body))
    for name, body in EXTRA.items():
        table.append((name, None, body))
    return table


def emit(sabotage: tuple[str, str] | None = None) -> str:
    """The fixture; `sabotage` is (pair name, "value" or a key of LOSES_A_REGISTER)."""
    parts = [HEADER]
    names = []
    for name, kind, body in pairs():
        if kind is None:
            decl = ""
            asm_body = jumps(body, "asm")
            ref_body = jumps(body, "ref")
        else:
            decl = KINDS[kind][0]
            asm_body = expand(body, kind, "asm")
            ref_body = expand(body, kind, "ref")
        parts.append("")
        planted = sabotage[1] if sabotage and sabotage[0] == name else None
        parts.append(function(name + "_asm", decl, asm_body, planted))
        parts.append("")
        parts.append(function(name + "_ref", decl, ref_body))
        names.append(name)
    parts.append("")
    parts.append("static const struct {")
    parts.append("    const char *name;")
    parts.append("    shape_fn *with_asm;")
    parts.append("    shape_fn *with_branch;")
    parts.append("} pairs[] = {")
    for name in names:
        parts.append(f'    {{ "{name}", {name}_asm, {name}_ref }},')
    parts.append("};")
    parts.append(FOOTER)
    return "\n".join(parts)


# ── running ─────────────────────────────────────────────────────────────────
RAW = "-DASM_GOTO_RAW_JUMP"
RUN_TIMEOUT_S = 20
LEVELS = ("-O0", "-O1", "-O2", "-O3", "-Os")
HOST_MATRIX = [
    (bits, level) + extra
    for extra in ((), (RAW,))
    for bits in ("-m64", "-m32")
    for level in LEVELS
]

# The planted fault "a register the caller owns is lost": the statement that
# loses it, and the mask `guarded_call` answers with (the last register of
# each target's table).
LOSES_A_REGISTER = {
    "register": ("xorl %r15d, %r15d", 0x20),
    "register-aarch64": ("mov x28, #0", 1 << (len(A64_GPR) - 1)),
    "register-riscv64": ("li s11, 0", 1 << (len(RV64_GPR) - 1)),
}

# ── the other targets ───────────────────────────────────────────────────────
# AArch64 and RISC-V run the same fixture under qemu-user.  Nothing but LCCC
# is needed to build it: the target's driver assembles and links a static
# program without a C library, around an entry stub and a `printf` for the
# conversions the fixture uses.  (A cross C library is a few hundred megabytes
# a lane has to install; a wrong edge of an asm goto is found without one.)
CROSS_ENTRY = {
    "aarch64": r"""
    .text
    .globl _start
_start:
    bl main
    mov w8, #93
    svc #0
    .globl sys_write
sys_write:
    mov w8, #64
    svc #0
    ret
    .globl memcpy
memcpy:
    mov x3, x0
1:  cbz x2, 2f
    ldrb w4, [x1], #1
    strb w4, [x3], #1
    sub x2, x2, #1
    b 1b
2:  ret
    .globl memset
memset:
    mov x3, x0
1:  cbz x2, 2f
    strb w1, [x3], #1
    sub x2, x2, #1
    b 1b
2:  ret
""",
    "riscv64": r"""
    .text
    .globl _start
_start:
    .option push
    .option norelax
    lla gp, __global_pointer$
    .option pop
    call main
    li a7, 93
    ecall
    .globl sys_write
sys_write:
    li a7, 64
    ecall
    ret
    .globl memcpy
memcpy:
    mv t0, a0
1:  beqz a2, 2f
    lbu t1, 0(a1)
    sb t1, 0(t0)
    addi a1, a1, 1
    addi t0, t0, 1
    addi a2, a2, -1
    j 1b
2:  ret
    .globl memset
memset:
    mv t0, a0
1:  beqz a2, 2f
    sb a1, 0(t0)
    addi t0, t0, 1
    addi a2, a2, -1
    j 1b
2:  ret
""",
}

CROSS_SUPPORT = r"""
/* printf for a program without a C library: %s %d %u %x %#x, a line at a time
 * through the write system call of the entry stub. */
long sys_write(long fd, const void *buffer, unsigned long count);

static char line[256];
static unsigned used;

static void put(char ch)
{
    if (used == sizeof line) {
        sys_write(1, line, used);
        used = 0;
    }
    line[used++] = ch;
}

static void put_number(unsigned long value, unsigned base)
{
    char digits[24];
    int count = 0;
    do {
        digits[count++] = "0123456789abcdef"[value % base];
        value /= base;
    } while (value);
    while (count)
        put(digits[--count]);
}

int printf(const char *format, ...)
{
    __builtin_va_list arguments;
    __builtin_va_start(arguments, format);
    for (; *format; format++) {
        int alternate = 0;
        if (*format != '%') {
            put(*format);
            continue;
        }
        if (*++format == '#') {
            alternate = 1;
            format++;
        }
        if (*format == 's') {
            const char *text = __builtin_va_arg(arguments, const char *);
            while (*text)
                put(*text++);
        } else if (*format == 'd') {
            long value = __builtin_va_arg(arguments, int);
            if (value < 0) {
                put('-');
                value = -value;
            }
            put_number((unsigned long)value, 10);
        } else if (*format == 'u') {
            put_number(__builtin_va_arg(arguments, unsigned), 10);
        } else if (*format == 'x') {
            unsigned value = __builtin_va_arg(arguments, unsigned);
            if (alternate && value) {
                put('0');
                put('x');
            }
            put_number(value, 16);
        } else {
            put('%');
            put(*format);
        }
    }
    __builtin_va_end(arguments);
    sys_write(1, line, used);
    used = 0;
    return 0;
}
"""


class CrossTarget:
    """One of the other targets: its LCCC driver and, where the host has one,
    the qemu-user that runs its programs."""

    DRIVERS = {"aarch64": "lccc-arm", "riscv64": "lccc-riscv"}

    def __init__(self, name: str, lccc: Path):
        import shutil

        self.name = name
        self.driver = lccc.resolve().parent / self.DRIVERS[name]
        self.emulator = shutil.which(f"qemu-{name}")
        self.support: list[str] | None = None

    def prepare(self, workdir: Path) -> str | None:
        """Build the entry stub and `printf` once per work directory; an error
        text if that fails."""
        if self.support is not None and all(Path(obj).is_file() for obj in self.support):
            return None
        self.support = None
        if not self.driver.is_file():
            return f"{self.driver} is not there (the target's driver sits beside --lccc)"
        entry = workdir / f"entry-{self.name}.s"
        entry.write_text(CROSS_ENTRY[self.name])
        support = workdir / f"support-{self.name}.c"
        support.write_text(CROSS_SUPPORT)
        objects = []
        for source in (entry, support):
            obj = source.with_suffix(".o")
            step = subprocess.run(
                [str(self.driver), "-O1", "-ffreestanding", "-w", "-c", str(source), "-o", str(obj)],
                capture_output=True, text=True, errors="replace",
            )
            if step.returncode != 0:
                return f"{source.name}: {(step.stderr or step.stdout).strip()[-600:]}"
            objects.append(str(obj))
        self.support = objects
        return None

    def build(self, flags: list[str], source: Path, exe: Path, workdir: Path):
        """The completed link step, or (None, error text)."""
        error = self.prepare(workdir)
        if error:
            return None, error
        step = subprocess.run(
            [str(self.driver), *flags, "-w", "-ffreestanding", "-nostdlib", "-static",
             str(source), *self.support, "-o", str(exe)],
            capture_output=True, text=True, errors="replace",
        )
        return step, None


def cross_matrix() -> list[tuple[str, str]]:
    return [(target, level) for target in CrossTarget.DRIVERS for level in LEVELS]


def build_and_run(
    compiler: list[str], flags: list[str], source: Path, workdir: Path, linker=None, cross=None
):
    """(status, detail): status is 'ok', 'mismatch', 'compile', 'crash' or 'skip'.

    With `cross` (a CrossTarget) the program is built by that target's driver
    and run under its emulator; 'skip' then means it was built and linked and
    the host has no emulator to run it.
    """
    exe = workdir / "eq"
    launcher: list[str] = []
    if cross:
        step, error = cross.build(flags, source, exe, workdir)
        if step is None:
            return "compile", error
        if step.returncode == 0 and not cross.emulator:
            return "skip", f"built and linked, not run: no qemu-{cross.name} on PATH"
        launcher = [cross.emulator]
    elif linker:
        obj = workdir / "eq.o"
        step = subprocess.run(
            compiler + flags + ["-w", "-c", str(source), "-o", str(obj)],
            capture_output=True, text=True, errors="replace",
        )
        if step.returncode == 0:
            step = subprocess.run(
                linker + [f for f in flags if f.startswith("-m")] + [str(obj), "-o", str(exe)],
                capture_output=True, text=True, errors="replace",
            )
    else:
        step = subprocess.run(
            compiler + flags + ["-w", str(source), "-o", str(exe)],
            capture_output=True, text=True, errors="replace",
        )
    if step.returncode != 0:
        return "compile", (step.stderr or step.stdout).strip()[-600:]
    try:
        # A run is 400 pairs times 32 inputs of straight-line code and short
        # loops: milliseconds.  Anything slower is a loop that lost its exit.
        run = subprocess.run(
            launcher + [str(exe)],
            capture_output=True, text=True, errors="replace", timeout=RUN_TIMEOUT_S,
        )
    except subprocess.TimeoutExpired:
        return "crash", f"still running after {RUN_TIMEOUT_S} s (a loop did not end)"
    except OSError as error:
        # no 32-bit loader on this host: nothing was learned, say so
        return "skip", str(error)
    if run.returncode == 0 and run.stdout.rstrip().endswith(": ok"):
        return "ok", run.stdout.strip()
    if run.returncode == 1 and "MISMATCH" in run.stdout:
        return "mismatch", run.stdout.strip()
    return "crash", f"exit {run.returncode}: {(run.stdout + run.stderr).strip()[-600:]}"


def run_matrix(
    compiler: list[str], label: str, source_text: str, matrix, verbose: bool, linker=None,
    cross: dict | None = None,
):
    """(rows run, rows failed, rows that could not run on this host).

    A row whose first element names a cross target (`cross`: name ->
    CrossTarget) is built by that target's driver from the rest of the row.
    """
    failures = 0
    ran = 0
    unrun = 0
    with tempfile.TemporaryDirectory(prefix="asm-goto-eq-") as tmp:
        work = Path(tmp)
        source = work / "asm_goto_equivalence.c"
        source.write_text(source_text)
        for row in matrix:
            flags = " ".join(row)
            target = (cross or {}).get(row[0])
            status, detail = build_and_run(
                compiler, list(row[1:] if target else row), source, work, linker, target
            )
            if status == "skip":
                unrun += 1
                print(f"SKIP {label} {flags}: {detail}")
                continue
            ran += 1
            if status == "ok":
                if verbose:
                    print(f"ok   {label} {flags}: {detail}")
                continue
            failures += 1
            print(f"FAIL {label} {flags} [{status}]")
            for line in detail.splitlines()[:40]:
                print("     " + line)
    return ran, failures, unrun


def self_test(
    compiler: list[str], cross: dict | None = None, host: bool = True, require_run: bool = False
) -> int:
    """The gate must see a fault, and name the pair that has it.

    Two are planted, one at a time, in the asm goto copy of one pair: it
    writes another value behind the jump, and it loses a register its caller
    owns.  The second is only planted where the host is x86-64, and once more
    for each cross target whose programs this host can run.  Without `host`
    (--cross-only) the cross targets get theirs and the host gets none.

    A planted fault has to be the one pair that differs, so each plant also
    says that the other pairs agree on its target at -O0.
    """
    import platform

    plants = []
    lost = "callee-saved registers lost, mask {:#x} with the asm goto, 0 with the branch"
    if host:
        # x = -1 jumps in both copies: the planted value is never written, so
        # the first input that differs is x = 0, where the asm copy returns 3
        # for 2.
        plants.append((None, "join_reg", "value", "x=0 y=0 n=0: asm goto 3 (G 70), branch 2 (G 70)"))
        if platform.machine() in ("x86_64", "AMD64"):
            plants.append(
                (None, "overwritten_glob", "register", lost.format(LOSES_A_REGISTER["register"][1]))
            )
    unrun = 0
    for name, target in (cross or {}).items():
        if target.emulator:
            kind = f"register-{name}"
            plants.append((target, "overwritten_glob", kind, lost.format(LOSES_A_REGISTER[kind][1])))
        else:
            # Building the plant would say nothing: it is the run that has to
            # report it.
            unrun += 1
            print(f"SKIP self-test {name}: no qemu-{name} on PATH to run its planted fault")
    if unrun and require_run:
        print(f"{unrun} planted faults were not run, and --require-cross-run says each has to be")
        return 1
    with tempfile.TemporaryDirectory(prefix="asm-goto-eq-self-") as tmp:
        work = Path(tmp)
        for target, victim, kind, expected in plants:
            source = work / f"planted-{kind}.c"
            source.write_text(emit(sabotage=(victim, kind)))
            flags = ["-O0"] if target else ["-m64", "-O0"]
            status, detail = build_and_run(compiler, flags, source, work, cross=target)
            reported = [line for line in detail.splitlines() if line.startswith("MISMATCH")]
            if (
                status != "mismatch"
                or len(reported) != 1
                or not reported[0].startswith(f"MISMATCH {victim} ")
                or expected not in reported[0]
                or "1 of " not in detail
            ):
                print(f"self-test FAILED: the planted {kind} fault in {victim} was reported as [{status}]")
                print(detail)
                return 1
    # and the generator is a function of its tables alone
    if emit() != emit():
        print("self-test FAILED: two runs of the generator differ")
        return 1
    count = len(pairs())
    expected_count = len(SHAPES) * len(KINDS) + len(EXTRA)
    if count != expected_count or len({name for name, _, _ in pairs()}) != count:
        print(f"self-test FAILED: {count} pairs, expected {expected_count} distinct ones")
        return 1
    if not plants:
        print(f"self-test: no fault could be planted on this host; {count} pairs")
        return 0
    print(f"self-test ok: {len(plants)} planted faults found where they were planted; {count} pairs")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--lccc", type=Path)
    ap.add_argument("--reference", help="another compiler to run the same matrix on")
    ap.add_argument("--reference-m32-linker", help="link -m32 objects of --reference with this driver")
    ap.add_argument("--emit", action="store_true")
    ap.add_argument("--write", action="store_true")
    ap.add_argument("--check-committed", action="store_true")
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument(
        "--self-test-only", action="store_true",
        help="--self-test and none of the rows: the planted faults alone",
    )
    ap.add_argument(
        "--only",
        help="comma separated rows, e.g. --only=-m64:-O2,-m32:-O1:" + RAW + ",aarch64:-O2",
    )
    ap.add_argument(
        "--cross", action="store_true",
        help="also build the fixture for AArch64 and RISC-V with the drivers beside --lccc "
        "and run it under qemu-user (built and linked only where there is no emulator)",
    )
    ap.add_argument("--cross-only", action="store_true", help="--cross without the host rows")
    ap.add_argument(
        "--require-cross-run", action="store_true",
        help="with --cross: a target this host cannot run is a failure, not a SKIP line",
    )
    ap.add_argument("-v", "--verbose", action="store_true")
    args = ap.parse_args()
    args.cross |= args.cross_only
    args.self_test |= args.self_test_only
    if args.require_cross_run and not args.cross:
        ap.error("--require-cross-run needs --cross")
    if args.cross and not args.lccc:
        ap.error("--cross needs --lccc: the cross drivers are found beside it")

    text = emit()
    if args.emit:
        sys.stdout.write(text)
        return 0
    if args.write:
        FIXTURE.write_text(text)
        print(f"wrote {FIXTURE.relative_to(HERE.parents[1])}: {len(pairs())} pairs")
        return 0

    rc = 0
    did = False
    if args.check_committed:
        did = True
        if not FIXTURE.is_file() or FIXTURE.read_text() != text:
            print(f"{FIXTURE.name} is not what the generator writes; run --write")
            rc = 1
        else:
            print(f"{FIXTURE.name} matches the generator ({len(pairs())} pairs)")

    cross = {name: CrossTarget(name, args.lccc) for name in CrossTarget.DRIVERS} if args.cross else {}
    matrix = ([] if args.cross_only else HOST_MATRIX) + (cross_matrix() if args.cross else [])
    if args.only:
        matrix = [tuple(item.split(":")) for item in args.only.split(",")]
        for row in matrix:
            if row[0] in CrossTarget.DRIVERS and not args.cross:
                ap.error(f"--only names the cross target {row[0]}: give --cross")
    host_rows = [row for row in matrix if row[0] not in CrossTarget.DRIVERS]

    if args.self_test:
        did = True
        compiler = [str(args.lccc)] if args.lccc else [args.reference or os.environ.get("CC", "cc")]
        rc |= self_test(
            compiler, cross, host=not args.cross_only, require_run=args.require_cross_run
        )

    if args.reference and not args.self_test_only:
        did = True
        ran, failures, _ = run_matrix(
            [args.reference], args.reference, text,
            [row for row in host_rows if row[0] == "-m64" or not args.reference_m32_linker],
            args.verbose,
        )
        if args.reference_m32_linker:
            ran32, failures32, _ = run_matrix(
                [args.reference], args.reference, text,
                [row for row in host_rows if row[0] == "-m32"], args.verbose,
                linker=[args.reference_m32_linker],
            )
            ran, failures = ran + ran32, failures + failures32
        print(f"{args.reference}: {ran} rows, {failures} failed")
        rc |= int(failures != 0)

    if args.lccc and not args.self_test_only:
        did = True
        ran, failures, unrun = run_matrix(
            [str(args.lccc)], "lccc", text, matrix, args.verbose, cross=cross
        )
        print(
            f"asm goto equivalence: {len(pairs())} pairs, {ran} rows, {failures} failed"
            + (f", {unrun} not run" if unrun else "")
        )
        # A cross row without its emulator was still built and linked; a host
        # row that cannot start says nothing at all.
        if ran == 0 and host_rows:
            print("no row could run on this host")
            rc = 1
        if unrun and args.require_cross_run:
            print(f"{unrun} rows were not run, and --require-cross-run says each has to be")
            rc = 1
        rc |= int(failures != 0)

    if not did:
        ap.error("nothing to do: give --lccc, --reference, --self-test, --check-committed, --emit or --write")
    return rc


if __name__ == "__main__":
    sys.exit(main())
