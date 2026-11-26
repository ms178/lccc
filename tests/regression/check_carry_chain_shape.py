#!/usr/bin/env python3
"""The code lccc emits for the adders of carry_chain_adders.c, pinned.

The fixture proves that the adders compute the right sums. This gate reads
the assembly (x86-64, -O2) and holds, for each of its functions:

* **the chain**: how many `adc`/`sbb` instructions there are, exactly. A
  full adder that the optimizer recognised is one; a function the
  optimizer must leave alone (three words added with their carries counted,
  a minuend that is the bit, a signed comparison) has as many as it has
  today, usually none.
* **the carry stays in the flag**: how many `setc`/`setb`/`bt` instructions
  stand between the first and the last `adc`/`sbb` of the function. In a
  straight chain of limbs that is none: `add; adc; adc; adc`, with no
  boolean made and tested again in between.
* **the size**: no more instructions than today (the number is a ceiling,
  lowered when the code gets better).

`SHAPES` is that table; `--print-table` prints it from the compiler at hand.

A limb loop is two loops since `limb_unroll`: four limbs a trip, and the
loop as it was for the rest. Between the two the carry is a boolean once
(one `setc`, one `bt`), and that is what their rows allow. What the split is
for is held apart, in `FOUR_A_TRIP`:

* **four links in the flag**: the longest run of `adc`/`sbb` in straight
  line code -- no label, no branch and no `setc`/`bt` between one and the
  next -- is four, exactly;
* **the flag round the loop**: the loop that holds those four (the lines
  from a label to the branch back to it) makes no boolean of the carry at
  all, neither a `setc` at its end nor a `bt` at its start, and has
  nothing in it that writes the carry flag but the four: it is moves,
  `lea`, the links, and a `dec` in front of the branch.

With the passes switched off the same reading must find that gone, which is
the negative control: without `carry` at least `MUST_BREAK` functions lose
their pin, without `limb_unroll` every function of `FOUR_A_TRIP` loses its
four links, and without the `carry_loop` peephole every one of them has
the boolean in its loop again.
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
FIXTURE = ROOT / "tests/regression/carry_chain_adders.c"
PREFIXES = ("hdr_", "bi_", "cmp_", "lib_", "two_")

# function: (adc + sbb, carry traffic inside the chain, instruction ceiling)
SHAPES: dict[str, tuple[int, int, int]] = {
    "hdr_add64": (1, 0, 12),
    "hdr_sub64": (1, 0, 12),
    "hdr_add32": (1, 0, 12),
    "hdr_sub32": (1, 0, 12),
    "hdr_carry_only": (1, 0, 9),
    "hdr_word_only": (0, 0, 7),
    "hdr_add32_wide": (1, 0, 14),
    "hdr_add_one": (1, 0, 11),
    "hdr_add_minus_one": (1, 0, 11),
    "hdr_sub_big": (1, 0, 12),
    "hdr_add32_all_ones": (1, 0, 11),
    "hdr_add_carry_set": (1, 0, 8),
    "hdr_sub_borrow_set": (1, 0, 8),
    "hdr_double": (1, 0, 11),
    "hdr_negate": (0, 0, 17),
    "hdr_from_constant": (1, 0, 12),
    "hdr_add4": (3, 0, 13),
    "hdr_sub4": (3, 0, 16),
    "hdr_add3_carry_in": (3, 0, 17),
    "hdr_add4_32": (3, 0, 16),
    "hdr_add4_counted": (4, 2, 28),
    "hdr_mixed": (3, 0, 16),
    "hdr_interrupted": (2, 3, 39),
    "hdr_in_place": (3, 0, 16),
    "hdr_addn": (5, 2, 43),
    "hdr_subn": (5, 2, 43),
    "hdr_addn_pairs": (2, 2, 33),
    "hdr_branchy": (2, 2, 37),
    "bi_addc": (0, 0, 12),
    "bi_subc": (0, 0, 12),
    "bi_addc32": (0, 0, 12),
    "bi_subc32": (0, 0, 12),
    "bi_add4": (3, 0, 16),
    "bi_sub4": (3, 0, 16),
    "bi_add4_32": (3, 0, 16),
    "cmp_add": (1, 0, 8),
    "cmp_add_other": (1, 0, 8),
    "cmp_sub": (1, 0, 11),
    "cmp_sub_grew": (1, 0, 11),
    "cmp_add32": (1, 0, 11),
    "cmp_sub32": (1, 0, 11),
    "cmp_add_masked": (1, 0, 10),
    "cmp_add_word": (0, 0, 14),
    "cmp_sub_word": (0, 0, 16),
    "cmp_add_loaded": (0, 0, 15),
    "cmp_add_logical": (1, 0, 8),
    "cmp_add_plus": (1, 0, 8),
    "cmp_add_xor": (1, 0, 8),
    "cmp_not_an_adder": (0, 0, 18),
    "cmp_signed": (0, 0, 17),
    "cmp_add_shared": (0, 0, 18),
    "cmp_add4": (3, 0, 16),
    "cmp_subn": (5, 2, 43),
    "cmp_addn32": (5, 2, 43),
    "cmp_add_carry_first": (1, 0, 8),
    "cmp_sub_borrow_first": (1, 0, 11),
    "cmp_sub_both_grew": (1, 0, 11),
    "cmp_sub_borrow_ahead": (1, 0, 11),
    "cmp_sub_of_sum": (1, 0, 11),
    "cmp_sub_of_sum_word": (1, 0, 9),
    "cmp_add_logical_int": (1, 0, 12),
    "cmp_add_plus_word": (0, 0, 13),
    "cmp_add_xor_word": (0, 0, 14),
    "cmp_sub_plus_word": (0, 0, 15),
    "cmp_sub_from_bit": (0, 0, 18),
    "lib_gmp_add_n": (5, 2, 60),
    "lib_gmp_sub_n": (5, 2, 60),
    "lib_mini_add_n": (5, 2, 43),
    "lib_mini_sub_n": (5, 2, 43),
    "lib_ossl_add_words": (5, 2, 63),
    "lib_ossl_sub_words": (0, 0, 30),
    "lib_i128_add_n": (5, 2, 43),
    "lib_i128_sub_n": (5, 2, 43),
    "lib_i128_add3_n": (0, 0, 30),
    "lib_i128_sub_neg_n": (5, 2, 43),
    "lib_i128_sub_top_n": (5, 2, 43),
    "lib_i128_sub_ne_n": (5, 2, 46),
    "lib_i128_sub_signed_n": (5, 2, 46),
    "lib_guarded_i128_sub_n": (5, 2, 67),
    "lib_blst_add_n": (5, 2, 44),
    "lib_blst_sub_n": (5, 2, 44),
    "lib_fiat_add4": (3, 0, 16),
    "lib_fiat_sub4": (3, 0, 16),
    "lib_ovf_add_n": (5, 2, 43),
    "lib_ovf_sub_n": (5, 2, 43),
    "lib_bool_add_n": (5, 2, 47),
    "lib_bool_sub_n": (5, 2, 47),
    "lib_int_add_n": (5, 2, 43),
    "lib_while_sub_n": (5, 2, 61),
    "lib_topbit_add_n": (5, 2, 51),
    "lib_guarded_add_n": (5, 1, 71),
    "lib_guarded_sub_n": (5, 1, 71),
    "lib_count_n": (0, 0, 35),
    "lib_count_carried_n": (0, 0, 27),
    "two_add_ssaaaa": (1, 0, 7),
    "two_sub_ddmmss": (1, 0, 8),
    "two_acc_add": (1, 0, 6),
    "two_acc_sub": (1, 0, 8),
    "two_acc3": (2, 0, 9),
    "two_stored_over_addend": (1, 0, 7),
    "two_stored_over_minuend": (1, 0, 8),
    "two_sum_in_place": (0, 0, 6),
}
# How many of them the pass is responsible for, at the least.
MUST_BREAK = 50

# The limb loops: `limb_unroll` splits each, and the main loop's four links
# pass the carry on in the flag.
FOUR_A_TRIP = (
    "hdr_addn",
    "hdr_subn",
    "cmp_subn",
    "cmp_addn32",
    "lib_gmp_add_n",
    "lib_gmp_sub_n",
    "lib_mini_add_n",
    "lib_mini_sub_n",
    "lib_ossl_add_words",
    "lib_i128_add_n",
    "lib_ovf_add_n",
    "lib_ovf_sub_n",
    "lib_bool_add_n",
    "lib_bool_sub_n",
    # A counter the test reads narrowed; a count stepped before it is
    # asked; a carry in that is the top bit of a word.
    "lib_int_add_n",
    "lib_while_sub_n",
    "lib_topbit_add_n",
    # Behind a guard: the main loop stands behind the function's `ret` and
    # is entered by a jump, in front of which the carry goes into the flag.
    "lib_guarded_add_n",
    "lib_guarded_sub_n",
    # The double word: the borrow read out of the high half of the
    # difference, each way a source reads it, and blst's two loops.
    "lib_i128_sub_n",
    "lib_i128_sub_neg_n",
    "lib_i128_sub_top_n",
    "lib_i128_sub_ne_n",
    "lib_i128_sub_signed_n",
    "lib_guarded_i128_sub_n",
    "lib_blst_add_n",
    "lib_blst_sub_n",
)
LINKS_A_TRIP = 4


LINK = re.compile(r"(adc|sbb)[bwlq]?\s")
CARRY_AS_A_BOOLEAN = re.compile(r"(setc|setb|setnae|bt[lq]?)\s")
LEAVES_THE_LINE = re.compile(r"(j[a-z]+|call[q]?|ret[q]?|loop[a-z]*)\b")
# In the main loop of a limb loop: the lines that write no flag, and the
# count that writes every flag but the carry.
LEAVES_THE_FLAGS = re.compile(r"(mov[a-z]*|lea[lq])\s")
KEEPS_THE_CARRY_COUNTING = re.compile(r"dec[lq]\s")


def read_functions(assembly: str) -> dict[str, list[str]]:
    """{function: its lines, labels included}"""
    functions: dict[str, list[str]] = {}
    current = None
    for line in assembly.splitlines():
        head = re.match(r"^([A-Za-z_]\w*):", line)
        text = line.strip()
        if head:
            current = functions.setdefault(head.group(1), [])
        elif text.startswith(".size"):
            current = None
        elif current is not None and text and not text.startswith("#"):
            if text.endswith(":") or not text.startswith("."):
                current.append(text)
    return {name: body for name, body in functions.items() if name.startswith(PREFIXES)}


def read_shapes(assembly: str) -> dict[str, tuple[int, int, int]]:
    """{function: (adc + sbb, carry traffic between the first and the last
    of them, instructions)}"""
    shapes = {}
    for name, lines in read_functions(assembly).items():
        body = [text for text in lines if not text.endswith(":")]
        links = [i for i, text in enumerate(body) if LINK.match(text)]
        inside = body[links[0]:links[-1]] if links else []
        traffic = sum(1 for text in inside if CARRY_AS_A_BOOLEAN.match(text))
        shapes[name] = (len(links), traffic, len(body))
    return shapes


def read_runs(assembly: str) -> dict[str, int]:
    """{function: the most `adc`/`sbb` in a row, with no label, no branch
    and no `setc`/`bt` between one and the next}"""
    runs = {}
    for name, lines in read_functions(assembly).items():
        longest = run = 0
        for text in lines:
            if LINK.match(text):
                run += 1
                longest = max(longest, run)
            elif text.endswith(":") or CARRY_AS_A_BOOLEAN.match(text) or LEAVES_THE_LINE.match(text):
                run = 0
        runs[name] = longest
    return runs


def compile_assembly(lccc: str, env: dict[str, str]) -> str:
    with tempfile.TemporaryDirectory(prefix="lccc-carry-shape-") as tmp:
        out = Path(tmp) / "adders.s"
        subprocess.run([lccc, "-O2", "-S", str(FIXTURE), "-o", str(out)], check=True,
                       env={**os.environ, **env})
        return out.read_text()


def compile_shapes(lccc: str, env: dict[str, str]) -> dict[str, tuple[int, int, int]]:
    return read_shapes(compile_assembly(lccc, env))


def read_loops(assembly: str) -> dict[str, tuple[int, int, int]]:
    """{function: (adc + sbb, setc + bt, lines that may write the carry flag
    besides) of its loop with the most links, a loop being the straight
    line from a label to the branch back to it}"""
    loops = {}
    for name, lines in read_functions(assembly).items():
        best = (0, 0, 0)
        label, body = None, []
        for text in lines:
            back = re.match(r"j[a-z]+\s+(\S+)$", text)
            if text.endswith(":"):
                label, body = text[:-1], []
            elif back and back.group(1) == label:
                links = [at for at, line in enumerate(body) if LINK.match(line)]
                traffic = sum(1 for line in body if CARRY_AS_A_BOOLEAN.match(line))
                # Besides the links and the lines that write no flag there
                # is the count, a `dec` behind the last link, and nothing.
                rest = [at for at, line in enumerate(body) if not (LINK.match(line) or LEAVES_THE_FLAGS.match(line))]
                counted = (len(rest) == 1 and bool(links) and rest[0] > links[-1]
                           and KEEPS_THE_CARRY_COUNTING.match(body[rest[0]]) is not None)
                best = max(best, (len(links), -traffic, -(len(rest) - counted)))
                label, body = None, []
            elif LEAVES_THE_LINE.match(text):
                label, body = None, []
            else:
                body.append(text)
        loops[name] = (best[0], -best[1], -best[2])
    return loops


def out_of_the_flag(loops: dict[str, tuple[int, int, int]]) -> list[str]:
    """The functions of FOUR_A_TRIP whose main loop makes a boolean of its
    carry, writes the carry flag with something else, or is not there."""
    problems = []
    for name in FOUR_A_TRIP:
        links, traffic, others = loops.get(name, (0, 0, 0))
        if (links, traffic, others) != (LINKS_A_TRIP, 0, 0):
            problems.append(f"{name}: the loop has {links} adc/sbb, {traffic} setc/bt and {others} lines that may "
                            f"write the carry flag, pinned {LINKS_A_TRIP}, 0 and 0")
    return problems


def short_trips(runs: dict[str, int]) -> list[str]:
    """The functions of FOUR_A_TRIP that do not have four links in a row."""
    return [f"{name}: {runs.get(name, 0)} adc/sbb in a row, pinned {LINKS_A_TRIP}"
            for name in FOUR_A_TRIP if runs.get(name, 0) != LINKS_A_TRIP]


def differences(shapes: dict[str, tuple[int, int, int]]) -> list[str]:
    problems = []
    for name, (links, traffic, ceiling) in SHAPES.items():
        if name not in shapes:
            problems.append(f"{name}: not in the assembly")
            continue
        got_links, got_traffic, size = shapes[name]
        if got_links != links:
            problems.append(f"{name}: {got_links} adc/sbb, pinned {links}")
        elif got_traffic > traffic:
            problems.append(f"{name}: {got_traffic} setc/bt inside the chain, pinned {traffic}")
        elif size > ceiling:
            problems.append(f"{name}: {size} instructions, ceiling {ceiling}")
    problems += [f"{name}: not pinned (add it to SHAPES)" for name in sorted(set(shapes) - set(SHAPES))]
    return problems


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--lccc", default=str(ROOT / "target/fastbuild/lccc"))
    parser.add_argument("--print-table", action="store_true", help="print SHAPES as this compiler has it")
    args = parser.parse_args()
    assembly = compile_assembly(args.lccc, {})
    shapes = read_shapes(assembly)
    if args.print_table:
        for name, (links, traffic, size) in shapes.items():
            print(f'    "{name}": ({links}, {traffic}, {size}),')
        return 0
    problems = differences(shapes) + short_trips(read_runs(assembly)) + out_of_the_flag(read_loops(assembly))
    problems += [f"{name}: in FOUR_A_TRIP and not in SHAPES" for name in FOUR_A_TRIP if name not in SHAPES]
    # The gate itself: without the passes, the pins must not hold.
    broken = differences(compile_shapes(args.lccc, {"CCC_DISABLE_PASSES": "carry"}))
    if len(broken) < MUST_BREAK:
        problems.append(f"negative control: with the pass off only {len(broken)} functions lose their pin, "
                        f"at least {MUST_BREAK} expected")
    whole = read_runs(compile_assembly(args.lccc, {"CCC_DISABLE_PASSES": "limb_unroll"}))
    unsplit = len(short_trips(whole))
    if unsplit != len(FOUR_A_TRIP):
        problems.append(f"negative control: with limb_unroll off only {unsplit} of {len(FOUR_A_TRIP)} limb loops "
                        f"lose their four links")
    parked = len(out_of_the_flag(read_loops(compile_assembly(args.lccc, {"CCC_PEEPHOLE_SKIP": "carry_loop"}))))
    if parked != len(FOUR_A_TRIP):
        problems.append(f"negative control: with the carry_loop peephole off only {parked} of {len(FOUR_A_TRIP)} "
                        f"limb loops make a boolean of their carry")
    for problem in problems:
        print(f"FAIL: {problem}")
    chains = sum(1 for links, _, _ in SHAPES.values() if links)
    print(f"carry-chain-shape: {len(SHAPES)} functions pinned, {chains} with adc/sbb "
          f"({sum(links for links, _, _ in SHAPES.values())} links), "
          f"{sum(size for _, _, size in shapes.values())} instructions; "
          f"{len(FOUR_A_TRIP)} limb loops with {LINKS_A_TRIP} links a trip and the carry in the flag round the loop; "
          f"{len(broken)} pins break without carry, {unsplit} trips without limb_unroll "
          f"and {parked} loops without carry_loop, "
          f"{'FAIL' if problems else 'PASS'}")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
