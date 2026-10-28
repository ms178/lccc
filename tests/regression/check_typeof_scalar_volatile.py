#!/usr/bin/env python3
"""Observable scalar typeof accesses, with nonvolatile and scope controls.

Run by check_volatile_access_semantics.sh; --compiler may also select the
pinned GCC oracle. Assembly checks are deliberately limited to x86-64 and
straight-line fixtures, not a claim about all qualified typeof expressions.
"""
import argparse
import os
from pathlib import Path
import re
import subprocess
import tempfile

FIXTURE = Path(__file__).with_name("typeof_scalar_volatile.c")
OPT_LEVELS = ("0", "1", "2", "3", "s")
SYMBOLS = {
    "expr_discard": "from_expr",
    "expr_sum": "from_expr",
    "expr_stores": "from_expr",
    "type_discard": "from_type",
    "nested_discard": "from_nested",
    "cv_reads": "cv",
    "block_extern": "block_ext",
    "outer_volatile_reads": "outer_volatile",
}


def function_body(assembly: str, name: str) -> str:
    pattern = rf"(?ms)^{re.escape(name)}:\s*$(.*?)^[ \t]*\.size\s+{re.escape(name)}\b"
    match = re.search(pattern, assembly)
    if match is None:
        raise AssertionError(f"missing function body: {name}")
    return match.group(1)


def inspect(compiler: str, opt: str, assembly: str) -> list[str]:
    errors = []

    def expect(name: str, description: str, count: int, wanted: int, *, exact: bool = True) -> None:
        if (exact and count != wanted) or (not exact and count < wanted):
            op = "exactly" if exact else "at least"
            errors.append(f"{compiler} -O{opt} {name}: {count} {description}; expected {op} {wanted}")

    for name, symbol in SYMBOLS.items():
        count = len(re.findall(rf"(?<![\w]){symbol}\(%rip\)", function_body(assembly, name)))
        expect(name, f"references to {symbol}", count, 2, exact=False)
    indirect = function_body(assembly, "indirect")
    count = len(re.findall(r"(?m)^\s*mov\w*\s+\(%r[a-z0-9]+\),", indirect))
    expect("indirect", "volatile pointee loads", count, 2, exact=False)
    for name in ("block_local", "shadow", "parameter"):
        body = function_body(assembly, name)
        count = len(re.findall(r"(?m)^\s*(?:mov\w*|add\w*)\s+-?\d*\(%r(?:sp|bp)\),", body))
        expect(name, "stack reads", count, 2, exact=False)
    if opt in ("2", "3", "s"):
        for name, symbols in {
            "unqual_discard": ("unqual_expr", "unqual_type", "nested_unqual"),
            "pointer_discard": ("pointer_copy",),
        }.items():
            body = function_body(assembly, name)
            for symbol in symbols:
                count = len(re.findall(rf"(?<![\w]){symbol}\(%rip\)", body))
                expect(name, f"nonvolatile references to {symbol}", count, 0)
        body = function_body(assembly, "shadow_nonvolatile")
        count = len(re.findall(r"(?m)^\s*(?:mov\w*|add\w*)\s+-?\d*\(%r(?:sp|bp)\),", body))
        expect("shadow_nonvolatile", "nonvolatile stack reads", count, 0)
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", default=os.environ.get("CCC", "target/fastbuild/lccc"))
    args = parser.parse_args()
    errors = []
    with tempfile.TemporaryDirectory(prefix="lccc-typeof-volatile-") as directory:
        output = Path(directory) / "probe.s"
        for opt in OPT_LEVELS:
            command = [args.compiler, f"-O{opt}", "-S", str(FIXTURE), "-o", str(output)]
            result = subprocess.run(command, text=True, capture_output=True, check=False)
            if result.returncode:
                errors.append(f"{' '.join(command)} failed:\n{result.stderr}")
                continue
            try:
                errors.extend(inspect(args.compiler, opt, output.read_text()))
            except AssertionError as error:
                errors.append(f"{args.compiler} -O{opt}: {error}")
    if errors:
        for error in errors:
            print("FAIL:", error)
        return 1
    print(f"scalar typeof volatile access: PASS ({args.compiler}; -O0/-O1/-O2/-O3/-Os)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
