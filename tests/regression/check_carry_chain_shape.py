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

With the pass switched off (`CCC_DISABLE_PASSES=carry`) the same reading
must find the chains gone: that is the negative control, and it fails the
gate if fewer than `MUST_BREAK` functions lose their pin.
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
    "hdr_addn": (1, 0, 16),
    "hdr_subn": (1, 0, 16),
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
    "cmp_subn": (1, 0, 16),
    "cmp_addn32": (1, 0, 16),
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
    "lib_gmp_add_n": (1, 0, 16),
    "lib_gmp_sub_n": (1, 0, 16),
    "lib_mini_add_n": (1, 0, 16),
    "lib_mini_sub_n": (1, 0, 16),
    "lib_ossl_add_words": (1, 0, 22),
    "lib_ossl_sub_words": (0, 0, 30),
    "lib_i128_add_n": (1, 0, 16),
    "lib_i128_sub_n": (1, 0, 21),
    "lib_i128_add3_n": (0, 0, 30),
    "lib_ovf_add_n": (1, 0, 16),
    "lib_ovf_sub_n": (1, 0, 16),
    "lib_bool_add_n": (1, 0, 16),
    "lib_bool_sub_n": (1, 0, 16),
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


def read_shapes(assembly: str) -> dict[str, tuple[int, int, int]]:
    """{function: (adc + sbb, carry traffic between the first and the last
    of them, instructions)}"""
    functions: dict[str, list[str]] = {}
    current = None
    for line in assembly.splitlines():
        head = re.match(r"^([A-Za-z_]\w*):", line)
        text = line.strip()
        if head:
            current = functions.setdefault(head.group(1), [])
        elif text.startswith(".size"):
            current = None
        elif current is not None and text and not text.startswith((".", "#")) and not text.endswith(":"):
            current.append(text)
    shapes = {}
    for name, body in functions.items():
        if not name.startswith(PREFIXES):
            continue
        links = [i for i, text in enumerate(body) if re.match(r"(adc|sbb)[bwlq]?\s", text)]
        inside = body[links[0]:links[-1]] if links else []
        traffic = sum(1 for text in inside if re.match(r"(setc|setb|setnae|bt[lq]?)\s", text))
        shapes[name] = (len(links), traffic, len(body))
    return shapes


def compile_shapes(lccc: str, env: dict[str, str]) -> dict[str, tuple[int, int, int]]:
    with tempfile.TemporaryDirectory(prefix="lccc-carry-shape-") as tmp:
        out = Path(tmp) / "adders.s"
        subprocess.run([lccc, "-O2", "-S", str(FIXTURE), "-o", str(out)], check=True,
                       env={**os.environ, **env})
        return read_shapes(out.read_text())


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
    shapes = compile_shapes(args.lccc, {})
    if args.print_table:
        for name, (links, traffic, size) in shapes.items():
            print(f'    "{name}": ({links}, {traffic}, {size}),')
        return 0
    problems = differences(shapes)
    # The gate itself: without the pass, the pins must not hold.
    broken = differences(compile_shapes(args.lccc, {"CCC_DISABLE_PASSES": "carry"}))
    if len(broken) < MUST_BREAK:
        problems.append(f"negative control: with the pass off only {len(broken)} functions lose their pin, "
                        f"at least {MUST_BREAK} expected")
    for problem in problems:
        print(f"FAIL: {problem}")
    chains = sum(1 for links, _, _ in SHAPES.values() if links)
    print(f"carry-chain-shape: {len(SHAPES)} functions pinned, {chains} with adc/sbb "
          f"({sum(links for links, _, _ in SHAPES.values())} links), "
          f"{sum(size for _, _, size in shapes.values())} instructions; "
          f"{len(broken)} pins break with the pass off, {'FAIL' if problems else 'PASS'}")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
