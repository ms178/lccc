#!/usr/bin/env python3
"""What a loop carries from one iteration to the next has a register.

Two programs of tests/benchmark/programs, compiled at -O2, and the one thing
in each that the register allocator lost when a value that is dead while a
loop runs stopped counting as a span of that loop
(`LiveRange::alive_within`):

zstd_count.c, `main`
    state = state * 1664525U + 1013904223U;
    The sum is read four times in the query loop's body, so it is no member
    of the phi's web and had no mark of a recurrence. With one register more
    in demand the recurrence floor took its register for the match pointer,
    and the slot sat between one query and the next: a store and a reload on
    the one chain the loop cannot overlap (+5 % to +7 % wall clock). Held
    here, for each of the two generators of the program: the sum is not
    stored to the frame where it is computed, and what the multiplication
    reads is not loaded from the frame on the way back to it.
    (`LiveRange::carried_round_loop`, `find_recurrence_floor_victim`.)

csv_field_sum.c, `sum_fields`
    cur = cur * 10 + (c - '0');   and   c = *++p   round the digit loop
    With no parameter in the frame the other block order won, and in it a
    temporary behind the loop held each phi's register while the value
    copied back was alive: two register moves at the bottom of every digit,
    12 instructions for 10. Held here: the digit loop has no register move
    and no more than ten instructions.
    (`rehome_phi_blockers`: the temporary takes the register the value
    copied back leaves.)

Neither is visible to .github/scripts/ci-codegen-gate.py: csv_field_sum is
not one of its workloads, and zstd_count's counts are the same with the
state in the frame (120 instructions, 6 pushes, 2 stack operands, against
121, 6 and 2).

The matchers are tried first on the code these changes replaced (kept below
as it was emitted), which they have to refuse, and on a line or two that
only look alike, which they have to pass.

    python3 tests/regression/check_loop_carried_homes.py --lccc target/fastbuild/lccc
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PROGRAMS = ROOT / "tests" / "benchmark" / "programs"

# Every name of a general register, to the 64-bit one.
REGISTERS: dict[str, str] = {}
for _full, _names in {
    "rax": "rax eax ax al ah",
    "rbx": "rbx ebx bx bl bh",
    "rcx": "rcx ecx cx cl ch",
    "rdx": "rdx edx dx dl dh",
    "rsi": "rsi esi si sil",
    "rdi": "rdi edi di dil",
    "rbp": "rbp ebp bp bpl",
    "rsp": "rsp esp sp spl",
}.items():
    for _name in _names.split():
        REGISTERS[_name] = _full
for _n in range(8, 16):
    for _suffix in ("", "d", "w", "b"):
        REGISTERS[f"r{_n}{_suffix}"] = f"r{_n}"

REG = r"%(\w+)"
FRAME = r"-?\d*\(%r[sb]p\)"
LOAD_FROM_FRAME = re.compile(rf"mov[lq]?\s+{FRAME},\s*{REG}$")
STORE_TO_FRAME = re.compile(rf"mov[lq]?\s+{REG},\s*{FRAME}$")
REGISTER_MOVE = re.compile(rf"mov[lq]?\s+{REG},\s*{REG}$")
MULTIPLY_STATE = re.compile(rf"imull\s+\$1664525,\s*{REG},\s*{REG}$")
# `state * 1664525 + 1013904223`, the sum in a register of its own or in
# the product's.
ADD_TO_STATE = re.compile(
    rf"(?:leal\s+1013904223\(%\w+\)|addl\s+\$1013904223),\s*{REG}$"
)
TIMES_TEN = re.compile(r"imulq\s+\$10,")
LABEL = re.compile(r"^(\.L\w+):$")
JUMP_IF = re.compile(r"j(?!mp)\w+\s+(\.L\w+)$")
JUMP = re.compile(r"j\w+\s+(\.L\w+)$")
# A move, an extension or an address into a register: how a value gets there.
INTO_REGISTER = re.compile(rf"(?:mov|lea)\w*\s+.*,\s*{REG}$")


def function(asm: str, name: str) -> list[str]:
    """The instructions and labels of `name`, stripped."""
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
            break
        if not line or line.startswith("#") or (line.startswith(".") and not LABEL.match(line)):
            continue
        lines.append(line)
    return lines


def state_in_frame(lines: list[str]) -> list[str]:
    """What is wrong with the homes of a generator's state, if anything.

    For every `imull $1664525, %A, ...`: the sum that follows it in the
    block is not stored to the frame, and on every jump back to the block's
    label the last thing written to A in the jumping block is no load from
    the frame.
    """
    wrong = []
    generators = 0
    for at, line in enumerate(lines):
        multiply = MULTIPLY_STATE.search(line)
        if not multiply:
            continue
        generators += 1
        multiplicand = REGISTERS[multiply.group(1)]
        # The sum, and what becomes of it until the block ends or it is
        # written again.
        total = None
        for behind in lines[at + 1 :]:
            if LABEL.match(behind):
                break
            if total is None:
                if m := ADD_TO_STATE.search(behind):
                    total = REGISTERS[m.group(1)]
                continue
            if (m := STORE_TO_FRAME.search(behind)) and REGISTERS.get(m.group(1)) == total:
                wrong.append(f"the sum is stored to the frame: {behind}")
            if (m := INTO_REGISTER.search(behind)) and REGISTERS.get(m.group(1)) == total:
                break
        if total is None:
            wrong.append(f"no `+ 1013904223` behind `{line}`: the program changed")
        # The way back.
        top = next((i for i in range(at, -1, -1) if LABEL.match(lines[i])), None)
        if top is None:
            wrong.append(f"no label in front of `{line}`: the program changed")
            continue
        label = LABEL.match(lines[top]).group(1)
        for back in range(at + 1, len(lines)):
            if not ((m := JUMP.search(lines[back])) and m.group(1) == label):
                continue
            for before in range(back - 1, -1, -1):
                if LABEL.match(lines[before]):
                    break
                written = INTO_REGISTER.search(lines[before])
                if written and REGISTERS.get(written.group(1)) == multiplicand:
                    if LOAD_FROM_FRAME.search(lines[before]):
                        wrong.append(
                            f"the multiplicand is loaded from the frame: {lines[before]}"
                        )
                    break
    if generators == 0:
        wrong.append("no `imull $1664525`: the program changed")
    return wrong


def digit_loop(lines: list[str]) -> list[str] | None:
    """The loop round `imulq $10`: from its label to the jump back to it."""
    at = next((i for i, line in enumerate(lines) if TIMES_TEN.search(line)), None)
    if at is None:
        return None
    top = next((i for i in range(at, -1, -1) if LABEL.match(lines[i])), None)
    if top is None:
        return None
    label = LABEL.match(lines[top]).group(1)
    for end in range(at, len(lines)):
        if LABEL.match(lines[end]):
            return None
        if (m := JUMP_IF.search(lines[end])) and m.group(1) == label:
            return lines[top + 1 : end + 1]
    return None


def copies_in_digit_loop(lines: list[str]) -> list[str]:
    """What is wrong with the digit loop, if anything."""
    loop = digit_loop(lines)
    if loop is None:
        return ["no loop of one block round `imulq $10`: the program changed"]
    wrong = [f"a register move in the digit loop: {line}" for line in loop if REGISTER_MOVE.search(line)]
    if len(loop) > 10:
        wrong.append(f"{len(loop)} instructions in the digit loop, 10 at most")
    return wrong


# The query loop of zstd_count's `main` with the state in the frame, and the
# digit loop of `sum_fields` with its two copies: as emitted before.
STATE_IN_FRAME = """
.LBB7:
    imull $1664525, %esi, %edx
    leal 1013904223(%rdx), %eax
    movl %eax, 4(%rsp)
    movl %eax, %r15d
    andl $1048319, %r15d
.LBB14:
    xorq %r15, %r10
    addq %r10, %r12
    addl $1, %edi
    movl 4(%rsp), %esi
    cmpl $131072, %edi
    jb .LBB7
"""
# With the state in registers. The offset that is kept in the frame instead
# is reloaded into the register the other generator multiplies: that is no
# finding, and neither is its store.
STATE_IN_REGISTER = """
.LBB2:
    imull $1664525, %r9d, %edi
    addl $1013904223, %edi
    movq %rdi, %r9
    cmpl $1048576, %r8d
    jb .LBB2
.LBB7:
    imull $1664525, %esi, %edx
    leal 1013904223(%rdx), %r14d
    movl %r14d, %eax
    andl $1048319, %eax
    movl %eax, 4(%rsp)
.LBB14:
    movl 4(%rsp), %r9d
    xorq %r9, %rdx
    addl $1, %edi
    movq %r14, %rsi
    cmpl $131072, %edi
    jb .LBB7
"""
DIGITS_WITH_COPIES = """
.LBB15:
    imulq $10, %r8, %r9
    movsbl %dil, %r11d
    leal -48(%r11), %eax
    movslq %eax, %rsi
    leaq (%r9, %rsi, 1), %r8
    leaq 1(%r10), %rdx
    movsbl (%rdx), %esi
    leaq -48(%rsi), %r11
    movq %rdx, %r10
    movq %rsi, %rdi
    cmpb $9, %r11b
    jbe .LBB15
"""
DIGITS_IN_PLACE = """
.LBB15:
    imulq $10, %r8, %r9
    movsbl %dil, %r11d
    leal -48(%r11), %eax
    movslq %eax, %rsi
    leaq (%r9, %rsi, 1), %r8
    addq $1, %r10
    movsbl (%r10), %edi
    leaq -48(%rdi), %r11
    cmpb $9, %r11b
    jbe .LBB15
"""


def self_test() -> list[str]:
    def body(text: str) -> list[str]:
        return [line.strip() for line in text.strip().splitlines()]

    failed = []
    found = state_in_frame(body(STATE_IN_FRAME))
    if len(found) != 2:
        failed.append(f"the state in the frame: 2 findings expected, {found}")
    if state_in_frame(body(STATE_IN_REGISTER)):
        failed.append("the state in a register is refused (another value's slot is not the state's)")
    found = copies_in_digit_loop(body(DIGITS_WITH_COPIES))
    if len(found) != 3:
        failed.append(f"the digit loop with copies: 3 findings expected, {found}")
    if copies_in_digit_loop(body(DIGITS_IN_PLACE)):
        failed.append("the digit loop without copies is refused (a sign extension is no move)")
    if not state_in_frame(body(DIGITS_IN_PLACE)) or not copies_in_digit_loop(body(STATE_IN_REGISTER)):
        failed.append("a function without the code looked for passes")
    return failed


def compile_to_asm(lccc: str, source: Path) -> str:
    with tempfile.TemporaryDirectory(prefix="lccc-carried-") as tmp:
        out = Path(tmp) / "out.s"
        done = subprocess.run(
            [lccc, "-O2", "-S", str(source), "-o", str(out)],
            capture_output=True,
            text=True,
            timeout=120,
        )
        if done.returncode != 0:
            raise SystemExit(f"FAIL: {source.name} does not compile:\n{done.stderr}")
        return out.read_text()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--lccc", required=True)
    args = parser.parse_args()

    failed = self_test()
    if failed:
        print("FAIL: the matchers do not hold on their own witnesses:")
        for line in failed:
            print(f"  {line}")
        return 1

    wrong = []
    main_lines = function(compile_to_asm(args.lccc, PROGRAMS / "zstd_count.c"), "main")
    wrong += [f"zstd_count.c main: {w}" for w in state_in_frame(main_lines)]
    sum_lines = function(compile_to_asm(args.lccc, PROGRAMS / "csv_field_sum.c"), "sum_fields")
    wrong += [f"csv_field_sum.c sum_fields: {w}" for w in copies_in_digit_loop(sum_lines)]
    if wrong:
        print("FAIL: a value a loop carries lost its register:")
        for line in wrong:
            print(f"  {line}")
        return 1
    loop = digit_loop(sum_lines) or []
    print(
        "loop-carried-homes: zstd_count's state stays in registers round the query loop; "
        f"csv_field_sum's digit loop is {len(loop)} instructions without a register move; "
        "5 witnesses, PASS"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
