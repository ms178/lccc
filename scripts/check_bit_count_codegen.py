#!/usr/bin/env python3
"""Fast PR791 assembly contracts; input directory comes from check_a9987_salvage.sh.

The negative opcode checks pin today's whole-network recognition boundary.
They are NOT a law forbidding future safe partial recognition: such a change
should update these shape assertions while retaining the executable witnesses
and hostile-edge/width IR tests. No v3 instructions are executed by this oracle.
"""
from pathlib import Path
import re
import sys


def body(path: Path, name: str) -> str:
    match = re.search(r"^" + name + r":\n(.*?)^\.size\s+" + name + r"\b",
                      path.read_text(), re.M | re.S)
    if match is None:
        raise SystemExit(f"FAIL: missing {name} in {path.name}")
    return match[1]


def count(text: str, opcode: str) -> int:
    return len(re.findall(r"^\s*(?:" + opcode + r")\s", text, re.M))


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit("FAIL: " + message)


def check(directory: Path) -> None:
    for arch, leading, trailing in [("x86-64", "bsrl", "bsfq"),
                                    ("x86-64-v3", "lzcntl", "tzcntq")]:
        path = directory / f"count-{arch}.s"
        for name, opcode in [("clz_full", leading), ("ctz_full", trailing)]:
            text = body(path, name)
            require(count(text, opcode) == 1, f"{arch} {name}: exactly one {opcode}")
            require(count(text, r"j\w+|cmov\w+") == 0,
                    f"{arch} {name}: redundant zero diamond/select")
        for name, opcode in [("clz_trunc", r"lzcnt\w*|bsr\w*"),
                             ("ctz_trunc", r"tzcnt\w*|bsf\w*")]:
            require(count(body(path, name), opcode) == 0,
                    f"{arch} {name}: unexpected whole-network recognition; inspect partial-fold proof")
        guards = directory / f"guards-{arch}.s"
        guarded_ctz = body(guards, "ctz_guard64")
        require(count(guarded_ctz, trailing) == 1, f"{arch} guarded CTZ: missing native count")
        require(count(guarded_ctz, r"or\w*") == 0,
                f"{arch} guarded CTZ: redundant high-bit safety OR")
        for name in ["clz_guard32", "clz_guard_live", "clz_guard_same_width",
                     "clz_guard_signed_widen", "clz_guard_zero_widen"]:
            text = body(guards, name)
            require(count(text, leading) == 1, f"{arch} {name}: missing native CLZ")
            require(not re.search(r"^\s*or\w*\s+\$1\s*,", text, re.M),
                    f"{arch} {name}: redundant OR1 under nonzero guard")
    print("PASS: typed CLZ32/CTZ64, safe-zero branch elimination, guarded OR1 removal, truncated near-misses")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("usage: check_bit_count_codegen.py ASSEMBLY_DIRECTORY")
    check(Path(sys.argv[1]))
