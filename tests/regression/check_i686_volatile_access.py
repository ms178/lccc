#!/usr/bin/env python3
"""i686 peepholes must not forward or delete volatile accesses."""
import argparse
from pathlib import Path
import re
import subprocess
import tempfile

FIXTURE = Path(__file__).with_name("i686_volatile_access.c")


def body(assembly: str, name: str) -> str:
    match = re.search(rf"(?ms)^{name}:\s*$(.*?)^[ \t]*\.size\s+{name}\b", assembly)
    if not match:
        raise AssertionError(f"missing function: {name}")
    return match.group(1)


def inspect(assembly: str, level: str) -> list[str]:
    errors = []
    stack_read = re.compile(r"(?m)^\s*(?:mov\w*|add\w*)\s+-?\d*\(%e(?:sp|bp)\),")
    stack_store = re.compile(r"(?m)^\s*mov\w*\s+(?:\$-?\d+|%[a-z]+),\s*-?\d*\(%e(?:sp|bp)\)")
    for name, minimum in (("direct", 2), ("parameter", 2), ("byte_reads", 2), ("wide_reads", 4)):
        actual = len(stack_read.findall(body(assembly, name)))
        if actual < minimum:
            errors.append(f"-O{level} {name}: {actual} stack reads, expected >= {minimum}")
    actual = len(stack_store.findall(body(assembly, "stores")))
    if actual < 2:
        errors.append(f"-O{level} stores: {actual} stack stores, expected >= 2")
    actual = len(re.findall(r"(?m)^\s*mov\w*\s+\(%e[a-z]+\),", body(assembly, "indirect")))
    if actual < 2:
        errors.append(f"-O{level} indirect: {actual} volatile loads, expected >= 2")
    if level in ("2", "3", "s") and stack_read.search(body(assembly, "plain_discard")):
        errors.append(f"-O{level} plain_discard: nonvolatile control still reads its stack")
    if "LCCC_VOLATILE_BEGIN" in assembly or "LCCC_VOLATILE_END" in assembly:
        errors.append(f"-O{level}: internal markers leaked into final assembly")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", default="target/fastbuild/lccc-i686")
    args = parser.parse_args()
    errors = []
    with tempfile.TemporaryDirectory(prefix="lccc-i686-volatile-") as directory:
        output = Path(directory) / "probe.s"
        for level in ("0", "1", "2", "3", "s"):
            cmd = [args.compiler, f"-O{level}", "-S", str(FIXTURE), "-o", str(output)]
            result = subprocess.run(cmd, text=True, capture_output=True, check=False)
            if result.returncode:
                errors.append(f"{' '.join(cmd)} failed: {result.stderr}")
                continue
            try:
                errors.extend(inspect(output.read_text(), level))
            except AssertionError as error:
                errors.append(f"-O{level}: {error}")
    if errors:
        for error in errors:
            print("FAIL:", error)
        return 1
    print(f"i686 volatile accesses: PASS ({args.compiler}; -O0/-O1/-O2/-O3/-Os)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
