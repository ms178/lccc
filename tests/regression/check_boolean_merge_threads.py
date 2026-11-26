#!/usr/bin/env python3
"""A way in that knows which way a branch goes takes it; nothing is written to be tested.

    static int smaller(...) { return a < b || (a == b && c <= d); }
    ...
    if (smaller(...)) j++;

Inlined, the value of `||` is a merge of the constant 1 and what the right
side computes, and the caller branches on it. If-conversion leaves that branch
(`if_convert::branch_is_decided_on_entry`), and behind the optimizer's rounds
the decided-branch threader sends the way in that brings the constant straight
to its arm (`jump_thread::thread_decided`). Before, if-conversion made the
branch a select, and the merge came out as (zlib_ng_huffman_heap.c,
`huffman_merge`)

        movl $1, %r12d              the path that knows it steps
    .LBB38:
        leal 1(%rdx), %r9d          computes the step,
        testq %r12, %r12            tests the 1 it has just written
        cmovnel %r9d, %edx          and selects

and, for the test that leaves the loop,

        xorl %r13d, %r13d
    .LBB42:
        testb %r13b, %r13b
        jne .LBB44

A condition built from conditions (`separator(c)` as `space(c) || c == ','`,
`space` an `||` of its own) is the same merge a level deeper; the threads are
taken a level a round, and the set fold that follows them makes what is left
of a character class one range test and one bit test. What the helper returns
stands between the merge and its branch -- nothing for `int`, `!= 0` for
`_Bool`, a narrowing cast for `unsigned char` -- and is read through: the
constant is folded through it (`jump_thread::constant_on_entry`), and the
branch on a conversion of a comparison is the branch on the comparison
(`simplify::truth_base`).

Three things are held, on x86-64 at -O2:

1. tests/regression/boolean_merge_threads.c prints what GCC 16.2's and Clang
   23.1.5's programs print, at -O0, -O1, -O2, -O3 and -Os, and at -O2 with the
   passes involved off one by one and together. Its `keep_*` functions are
   where a wrong answer would come from: a constant that is not 0 or 1 behind
   a narrowing cast (256 as an `unsigned char` is 0), a value read again
   behind the branch, a merge that is a loop's own test, a `continue`.

2. The functions of that file named below are no longer than today, and
   those over a character class test it with a bit test.

3. In `huffman_merge` of tests/benchmark/programs/zlib_ng_huffman_heap.c no
   register is tested that the code falling into the test has just loaded
   with a constant, and the function is no longer than today.

Negative controls: with CCC_DISABLE_PASSES=jump_thread_early every pin of 2
and 3 has to break, and with set_membership off every pin that asks for a bit
test. The matcher of 3 is tried first on the code above, kept below as it was
emitted, which it has to refuse, and on code that only looks alike, which it
has to pass.

    python3 tests/regression/check_boolean_merge_threads.py --lccc target/fastbuild/lccc
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
FIXTURE = ROOT / "tests" / "regression" / "boolean_merge_threads.c"
HUFFMAN = ROOT / "tests" / "benchmark" / "programs" / "zlib_ng_huffman_heap.c"

# What the fixture prints when compiled by GCC 16.2 (-O0, -O2, and -O3 with
# -funsigned-char) and by Clang 23.1.5 (-O2, and -O1 with -fsanitize=undefined
# -fsanitize-trap=all, which does not trap).
EXPECTED = "boolean merges 764046bd538df876"

# Function -> (the most instructions it may have, whether it tests a character
# class and has to do so with a bit test). Today's counts; with the early
# position of the threader off each of these breaks.
PINS = {
    "merge_count_before": (33, False),
    "merge_count_ordered": (44, False),
    "nest_skip_separators": (15, True),
    "nest_token_end": (12, True),
    "nest_skip_sparse": (15, True),
    "nest_skip_nested": (15, True),
    "nest_skip_sparse_bool": (15, True),
    "nest_skip_nested_byte": (15, True),
    "nest_count_punctuation": (27, True),
    "nest_sum_wanted": (23, False),
}
HUFFMAN_FUNCTION = "huffman_merge"
HUFFMAN_CEILING = 253

# The pass whose absence has to break every pin, and the one whose absence
# has to break every pin that asks for a bit test.
THREADER = "jump_thread_early"
SET_FOLD = "set_membership"

# The configurations whose output is compared: flags, and passes switched off.
CONFIGURATIONS = [
    (["-O0"], ""),
    (["-O1"], ""),
    (["-O2"], ""),
    (["-O3"], ""),
    (["-Os"], ""),
    (["-O2"], "jump_thread_early"),
    (["-O2"], "jump_thread"),
    (["-O2"], "set_membership"),
    (["-O2"], "range_fold"),
    (["-O2"], "set_membership,range_fold"),
    (["-O2"], "ifconv"),
    (["-O2"], "jump_thread,ifconv"),
    (["-O2", "-funsigned-char"], ""),
    (["-O2", "-fPIC"], ""),
]

# Every name of a general register, to the 64-bit one.
REGISTERS: dict[str, str] = {}
for _full, _names in {
    "rax": "rax eax ax al",
    "rbx": "rbx ebx bx bl",
    "rcx": "rcx ecx cx cl",
    "rdx": "rdx edx dx dl",
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

LABEL = re.compile(r"^[.\w]+:$")
TEST_OF_ITSELF = re.compile(r"test[bwlq]?\s+%(\w+),\s*%(\w+)$")
# `mov $K, %R`, or `xor %R, %R`.
CONSTANT_INTO = re.compile(r"(?:mov[bwlq]?\s+\$-?\w+|xor[lq]?\s+%(\w+)),\s*%(\w+)$")
# Anything else whose last operand is a register it writes.
WRITES = re.compile(r"^(?!cmp|test|push|j|call|ret|bt[lq]?\s)\w+\s+(?:.*,\s*)?%(\w+)$")
# Behind these nothing falls through, or everything may have changed.
NO_FALL_THROUGH = re.compile(r"^(?:jmp|ret|call)\b")
BIT_TEST = re.compile(r"^bt[lq]?\s")


def function(asm: str, name: str) -> list[str] | None:
    """The instructions and labels of `name`, stripped; None when absent."""
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
        if not line or line.startswith("#") or (line.startswith(".") and not LABEL.match(line)):
            continue
        lines.append(line)
    return None


def instructions(lines: list[str]) -> list[str]:
    return [line for line in lines if not LABEL.match(line)]


def tested_constants(lines: list[str]) -> list[str]:
    """Each `test %R, %R` that the code falling into it reaches with a
    constant in R: walking back over labels, the first instruction that
    writes R is `mov $K, %R` or `xor %R, %R`. A jump, a return or a call in
    between ends the walk: nothing falls through it, or R is not ours."""
    found = []
    for at, line in enumerate(lines):
        test = TEST_OF_ITSELF.match(line)
        if not test:
            continue
        register = REGISTERS.get(test.group(1))
        if register is None or REGISTERS.get(test.group(2)) != register:
            continue
        for back in range(at - 1, -1, -1):
            before = lines[back]
            if LABEL.match(before):
                continue
            if NO_FALL_THROUGH.match(before):
                break
            constant = CONSTANT_INTO.match(before)
            if (
                constant
                and REGISTERS.get(constant.group(2)) == register
                and (constant.group(1) is None or REGISTERS.get(constant.group(1)) == register)
            ):
                found.append(f"`{before}` falls into `{line}`")
                break
            written = WRITES.match(before)
            if written and REGISTERS.get(written.group(1)) == register:
                break
    return found


def pin_faults(lines: list[str], ceiling: int, bit_test: bool) -> list[str]:
    """What is wrong with a pinned function of the fixture, if anything."""
    body = instructions(lines)
    wrong = []
    if len(body) > ceiling:
        wrong.append(f"{len(body)} instructions, {ceiling} at most")
    if bit_test and not any(BIT_TEST.match(line) for line in body):
        wrong.append("no bit test: the class is tested member by member")
    return wrong


def huffman_faults(lines: list[str], ceiling: int) -> list[str]:
    """What is wrong with `huffman_merge`, if anything."""
    wrong = [f"a constant written to be tested: {found}" for found in tested_constants(lines)]
    count = len(instructions(lines))
    if count > ceiling:
        wrong.append(f"{count} instructions, {ceiling} at most")
    return wrong


# `huffman_merge` round the two merges of one sift level, as emitted before.
CONSTANTS_TESTED = """
.LBB36:
    movzwl (%rdi, %r10, 4), %r13d
    movzwl (%rdi, %r9, 4), %r12d
    cmpl %r12d, %r13d
    jge .LBB62
.LBB37:
    movl $1, %r12d
.LBB38:
    leal 1(%rdx), %r9d
    testq %r12, %r12
    cmovnel %r9d, %edx
.LBB39:
    cmpl %r8d, %r11d
    jl .LBB44
.LBB40:
    je .LBB61
.LBB41:
    xorl %r13d, %r13d
.LBB42:
    testb %r13b, %r13b
    jne .LBB44
"""

# Tests that only look alike: of a register loaded from memory, of one a
# call returned, of one written behind the constant, behind a jump, and a
# test of two registers.
NOT_CONSTANTS = """
.LBB1:
    movl $1, %r12d
    movl (%rdi), %r12d
    testl %r12d, %r12d
    jne .LBB9
.LBB2:
    xorl %eax, %eax
    call next
    testl %eax, %eax
    jne .LBB9
.LBB3:
    movl $1, %ecx
    addl %esi, %ecx
    testl %ecx, %ecx
    jne .LBB9
.LBB4:
    xorl %edx, %edx
    jmp .LBB6
.LBB5:
    testl %edx, %edx
    jne .LBB9
.LBB6:
    movl $7, %r8d
    testl %r8d, %r9d
    jne .LBB9
.LBB7:
    xorl %r10d, %r11d
    testl %r11d, %r11d
    jne .LBB9
"""


def self_test() -> list[str]:
    wrong = []
    kept = [line.strip() for line in CONSTANTS_TESTED.strip().splitlines()]
    found = tested_constants(kept)
    if len(found) != 2:
        wrong.append(f"the kept code has two tested constants, the matcher finds {len(found)}: {found}")
    found = tested_constants([line.strip() for line in NOT_CONSTANTS.strip().splitlines()])
    if found:
        wrong.append(f"the matcher takes for a tested constant: {found}")
    chain = ["cmpb $32, %r11b", "je .LBB8", "cmpb $9, %r11b", "je .LBB8", "ret"]
    mask = ["leaq -9(%r11), %r10", "cmpl $50, %r10d", "ja .LBB9", "btq %r10, %rcx", "jb .LBB8", "ret"]
    if not pin_faults(chain, 5, True):
        wrong.append("the matcher takes a chain of compares for a bit test")
    if pin_faults(mask, 6, True):
        wrong.append("the matcher refuses a range test and a bit test")
    if pin_faults(chain, 5, False):
        wrong.append("the matcher asks every function for a bit test")
    if not pin_faults(mask, 5, False):
        wrong.append("the matcher misses an instruction over the ceiling")
    return wrong


def run(command: list[str], disabled: str = "", timeout: int = 120) -> subprocess.CompletedProcess:
    env = dict(os.environ)
    env.pop("CCC_DISABLE_PASSES", None)
    if disabled:
        env["CCC_DISABLE_PASSES"] = disabled
    return subprocess.run(command, capture_output=True, text=True, timeout=timeout, env=env)


def assembly(lccc: str, source: Path, work: Path, disabled: str, flags: list[str]) -> str | None:
    out = work / f"{source.stem}.{disabled.replace(',', '+') or 'on'}.s"
    done = run([lccc, *flags, "-S", str(source), "-o", str(out)], disabled)
    if done.returncode != 0:
        print(f"FAIL: {source.name} does not compile ({disabled or 'default'}):\n{done.stderr}")
        return None
    return out.read_text()


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

    with tempfile.TemporaryDirectory(prefix="boolean-merge-") as tmp:
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
        for number, (flags, disabled) in enumerate(CONFIGURATIONS):
            what = " ".join(flags) + (f" without {disabled}" if disabled else "")
            binary = work / f"fixture{number}"
            done = run([args.lccc, *flags, str(FIXTURE), "-o", str(binary)], disabled)
            if done.returncode != 0:
                failures.append(f"{what}: the fixture does not compile:\n{done.stderr}")
                continue
            ran = run([str(binary)])
            if ran.returncode != 0 or ran.stdout.strip() != EXPECTED:
                failures.append(
                    f"{what}: exit {ran.returncode}, `{ran.stdout.strip()}`, expected `{EXPECTED}`"
                )

        # 2 and 3. The shapes, and the same with each pass off.
        held = 0
        broken = {THREADER: 0, SET_FOLD: 0}
        expected_broken = {THREADER: len(PINS) + 1, SET_FOLD: sum(bit for _, bit in PINS.values())}
        for disabled in ("", THREADER, SET_FOLD):
            fixture = assembly(args.lccc, FIXTURE, work, disabled, ["-O2"])
            huffman = assembly(args.lccc, HUFFMAN, work, disabled, ["-O2", "-std=gnu17"])
            if fixture is None or huffman is None:
                return 1
            checks = [
                (
                    name,
                    bit_test,
                    function(fixture, name),
                    lambda lines, c=ceiling, b=bit_test: pin_faults(lines, c, b),
                )
                for name, (ceiling, bit_test) in PINS.items()
            ]
            checks.append(
                (
                    HUFFMAN_FUNCTION,
                    False,
                    function(huffman, HUFFMAN_FUNCTION),
                    lambda lines: huffman_faults(lines, HUFFMAN_CEILING),
                )
            )
            for name, bit_test, lines, faults in checks:
                if lines is None:
                    failures.append(f"{name}: not in the assembly; the program changed")
                    continue
                wrong = faults(lines)
                if not disabled:
                    held += 1
                    failures += [f"{name}: {fault}" for fault in wrong]
                elif disabled == SET_FOLD and not bit_test:
                    continue
                elif wrong:
                    broken[disabled] += 1
                else:
                    failures.append(
                        f"{name}: holds with {disabled} off, so the pin does not measure the pass"
                    )

    for failure in failures:
        print(f"FAIL: {failure}")
    if failures:
        return 1
    print(
        f"PASS: {len(CONFIGURATIONS)} configurations print `{EXPECTED}`; "
        f"{held} functions pinned (no longer than today, a bit test for each character class, "
        f"no constant written to be tested); "
        f"{broken[THREADER]} of {expected_broken[THREADER]} pins break with {THREADER} off, "
        f"{broken[SET_FOLD]} of {expected_broken[SET_FOLD]} with {SET_FOLD} off"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
