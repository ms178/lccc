#!/usr/bin/env python3
"""A function with a jump table is analysed, position-fixed as well.

Where the code will not move (-fno-pic, -mcmodel=kernel, and a link the
driver knows to be an ordinary executable: `lccc prog.c -o prog`) a `switch`
is dispatched by one instruction through a table of addresses:

        jmpq *.Ljt_0(,%rax,8)
    .Ljt_0:
        .quad .LBB3
        .quad .LBB4

The peephole's liveness analysis (`FileLiveness::resolve_jump_tables`) knew
the position-independent form, `leaq .Ljt_0(%rip), %rcx; movslq
(%rcx,%rax,4), %rdx; addq %rcx, %rdx; jmpq *%rdx` over `.long .LBB3 -
.Ljt_0`, and took this one for a jump to an unknown place. One such jump
makes a function unanalysable, so no liveness-driven peephole ran in any
function that held a table, in exactly the builds a benchmark is and the
kernel is: 11 to 19 instructions too many in each function of
tests/regression/switch_table_liveness.c (71 for 57, 81 for 63, 107 for 88),
moves of a call's result that nothing reads among them.

Two things are held, on x86-64:

1. The fixture prints what GCC 16.2's and Clang 23.1.5's programs print:
   at -O0, -O1, -O2, -O3 and -Os, linked as the driver links by default
   (position-fixed), with -fPIE -pie and with -fno-pie -no-pie.

2. At -O2, -O3 and -Os, for each `dispatch_*` function: the position-fixed
   code (-fno-pie, and -mcmodel=kernel -fno-pic) has at least one table
   dispatch and is no longer than the position-independent code less the
   three instructions each dispatch saves. No function of the fixture names
   a global or a string, so the two forms differ in the dispatch and in
   nothing else, and the bound is exact when both are analysed alike.

The check of 2 is tried first on one function as it was emitted before
(kept below), which it has to refuse.

    python3 tests/regression/check_switch_table_liveness.py --lccc target/fastbuild/lccc
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
FIXTURE = ROOT / "tests" / "regression" / "switch_table_liveness.c"

# What the fixture prints when compiled by GCC 16.2 (-O0, -O2) and by Clang
# 23.1.5 (-O2, and -O1 with -fsanitize=undefined -fsanitize-trap=all).
EXPECTED = "switch tables bfc3ae850bd6180a"

FUNCTIONS = (
    "dispatch_build_values",
    "dispatch_run_program",
    "dispatch_count_tokens",
    "dispatch_combine",
    "dispatch_twice",
)

LEVELS = ("-O0", "-O1", "-O2", "-O3", "-Os")
LINKS = ((), ("-fPIE", "-pie"), ("-fno-pie", "-no-pie"))
SHAPE_LEVELS = ("-O2", "-O3", "-Os")
# The position-fixed code models: flags, and what to call them.
FIXED = (
    (("-fno-pie",), "-fno-pie"),
    (("-mcmodel=kernel", "-fno-pic"), "-mcmodel=kernel"),
)

LABEL = re.compile(r"^[.\w]+:$")
ABSOLUTE_DISPATCH = re.compile(r"^jmpq \*\.Ljt_\w+\(,%\w+,8\)$")
RELATIVE_DISPATCH = re.compile(r"^leaq \.Ljt_\w+\(%rip\), %\w+$")
# What the one-instruction dispatch saves: the table's address, the entry
# and the sum (`leaq`, `movslq`, `addq`) ahead of the jump.
SAVED_PER_DISPATCH = 3


def function(asm: str, name: str) -> list[str] | None:
    """The instructions of `name`, stripped; None when absent."""
    lines: list[str] = []
    inside = False
    for raw in asm.splitlines():
        line = raw.strip()
        if line == f"{name}:":
            inside = True
            continue
        if not inside:
            continue
        if line.startswith(".size"):
            return lines
        if not line or line.startswith("#") or line.startswith(".") or LABEL.match(line):
            continue
        lines.append(line)
    return None


def fixed_faults(fixed: list[str], independent: list[str]) -> list[str]:
    """What is wrong with a position-fixed function beside its
    position-independent form, if anything."""
    tables = sum(1 for line in fixed if ABSOLUTE_DISPATCH.match(line))
    relative = sum(1 for line in independent if RELATIVE_DISPATCH.match(line))
    if tables == 0:
        return ["no dispatch through an absolute table: the fixture or the code generator changed"]
    if relative != tables:
        return [f"{tables} absolute and {relative} relative dispatches: the two forms are not one function"]
    bound = len(independent) - SAVED_PER_DISPATCH * tables
    if len(fixed) > bound:
        return [
            f"{len(fixed)} instructions, {bound} at most "
            f"({len(independent)} position-independent, {tables} dispatch(es))"
        ]
    return []


# `dispatch_combine` at -O2 -fno-pie as it was emitted before (less the CFI
# directives and the markers): 71 instructions, beside 60
# position-independent ones (57 at most).
BEFORE_INDEPENDENT = 60
BEFORE = """
dispatch_combine:
    subq $8, %rsp
    movq %rsi, %r11
    movq %rdx, %r10
    movq %rcx, %r9
    movl %edi, %edi
    cmpl $9, %edi
    jae .LBB76
    jmpq *.Ljt_3(,%rdi,8)
.section .rodata
.align 8
.Ljt_3:
    .quad .LBB67
    .quad .LBB68
    .quad .LBB69
    .quad .LBB70
    .quad .LBB71
    .quad .LBB72
    .quad .LBB73
    .quad .LBB74
    .quad .LBB75
.section .text,"ax",@progbits
.LBB67:
    leaq (%r11, %r10, 1), %r8
    movq %r8, %rax
    addq $8, %rsp
    ret
.LBB68:
    movq %r11, %rdi
    subq %r9, %rdi
    movq %rdi, %rax
    addq $8, %rsp
    ret
.LBB69:
    movq %r10, %rdi
    imulq %r9, %rdi
    movq %rdi, %rax
    addq $8, %rsp
    ret
.LBB70:
    movq %r11, %rsi
    xorq %r10, %rsi
    addq %r9, %rsi
    movq %rsi, %rax
    addq $8, %rsp
    ret
.LBB71:
    movl %r10d, %r8d
    andl $31, %r8d
    movq %r11, %rdi
    movl %r8d, %ecx
    shlq %cl, %rdi
    movq %rdi, %rax
    addq $8, %rsp
    ret
.LBB72:
    movl %r11d, %edi
    andl $31, %edi
    movq %r9, %rsi
    movl %edi, %ecx
    shrq %cl, %rsi
    movq %rsi, %rax
    addq $8, %rsp
    ret
.LBB73:
    movl %r10d, %esi
    movq %r11, %rdi
    call pack
    movl %eax, %r10d
    movzbl %al, %r8d
    leaq (%r8, %r9), %rdi
    movq %rdi, %rax
    addq $8, %rsp
    ret
.LBB74:
    movq %r10, %rdi
    notq %rdi
    movq %r11, %rsi
    andq %rdi, %rsi
    movq %rsi, %rax
    addq $8, %rsp
    ret
.LBB75:
    movq %r11, %r8
    orq %r10, %r8
    xorq %r9, %r8
    movq %r8, %rax
    addq $8, %rsp
    ret
.LBB76:
    addq %r10, %r11
    addq %r9, %r11
    movq %r11, %rax
    addq $8, %rsp
    ret
.size dispatch_combine, .-dispatch_combine
"""


def self_test() -> list[str]:
    wrong = []
    before = function(BEFORE, "dispatch_combine")
    if before is None:
        return ["the kept function is not found by its name"]
    independent = ["leaq .Ljt_3(%rip), %rcx"] + ["nop"] * (BEFORE_INDEPENDENT - 1)
    if not fixed_faults(before, independent):
        wrong.append(
            f"the kept function ({len(before)} instructions beside {BEFORE_INDEPENDENT}) is accepted"
        )
    # The same function three instructions shorter than its other form: fine.
    if fixed_faults(before[: BEFORE_INDEPENDENT - SAVED_PER_DISPATCH], independent):
        wrong.append("a function at the bound is refused")
    if fixed_faults(before[: BEFORE_INDEPENDENT - SAVED_PER_DISPATCH + 1], independent) == []:
        wrong.append("a function one instruction over the bound is accepted")
    if not fixed_faults(["ret"], ["ret"]):
        wrong.append("a function without a table is accepted")
    if not fixed_faults(before[:20], ["nop"] * 60):
        wrong.append("two forms with different numbers of dispatches are accepted")
    return wrong


def run(command: list[str], timeout: int = 120) -> subprocess.CompletedProcess:
    return subprocess.run(command, capture_output=True, text=True, timeout=timeout)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    parser.add_argument("--lccc", default=str(ROOT / "target" / "fastbuild" / "lccc"))
    parser.add_argument(
        "--gcc",
        default=None,
        help="a reference compiler; given, its program has to print what is expected here",
    )
    args = parser.parse_args()

    failures = self_test()
    for failure in failures:
        print(f"FAIL: {failure}")
    if failures:
        return 1

    programs = 0
    held = 0
    with tempfile.TemporaryDirectory(prefix="switch-table-") as tmp:
        work = Path(tmp)

        # 1. The output.
        if args.gcc:
            binary = work / "reference"
            done = run([args.gcc, "-O2", str(FIXTURE), "-o", str(binary)])
            if done.returncode != 0:
                print(f"FAIL: {args.gcc} does not compile the fixture:\n{done.stderr}")
                return 1
            printed = run([str(binary)]).stdout.strip()
            if printed != EXPECTED:
                failures.append(f"{args.gcc} prints `{printed}`, expected here is `{EXPECTED}`")
        for level in LEVELS:
            for link in LINKS:
                what = " ".join((level, *link))
                binary = work / f"fixture{programs}"
                done = run([args.lccc, level, *link, str(FIXTURE), "-o", str(binary)])
                if done.returncode != 0:
                    failures.append(f"{what}: the fixture does not compile:\n{done.stderr}")
                    continue
                ran = run([str(binary)])
                programs += 1
                if ran.returncode != 0 or ran.stdout.strip() != EXPECTED:
                    failures.append(
                        f"{what}: exit {ran.returncode}, `{ran.stdout.strip()}`, expected `{EXPECTED}`"
                    )

        # 2. The shape.
        def assembly(flags: tuple[str, ...]) -> str | None:
            out = work / "fixture.s"
            done = run([args.lccc, *flags, "-S", str(FIXTURE), "-o", str(out)])
            if done.returncode != 0:
                failures.append(f"{' '.join(flags)}: the fixture does not compile:\n{done.stderr}")
                return None
            return out.read_text()

        for level in SHAPE_LEVELS:
            independent = assembly((level, "-fPIE"))
            if independent is None:
                continue
            for flags, model in FIXED:
                fixed = assembly((level, *flags))
                if fixed is None:
                    continue
                for name in FUNCTIONS:
                    ours, theirs = function(fixed, name), function(independent, name)
                    if ours is None or theirs is None:
                        failures.append(f"{level} {model}: {name} is not in the assembly")
                        continue
                    wrong = fixed_faults(ours, theirs)
                    failures += [f"{level} {model}: {name}: {fault}" for fault in wrong]
                    held += not wrong

    for failure in failures:
        print(f"FAIL: {failure}")
    if failures:
        return 1
    print(
        f"PASS: {programs} programs print `{EXPECTED}`; {held} position-fixed functions "
        f"({len(FUNCTIONS)} functions, {len(SHAPE_LEVELS)} levels, {len(FIXED)} code models) are "
        f"no longer than their position-independent form less {SAVED_PER_DISPATCH} "
        f"instructions a dispatch"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
