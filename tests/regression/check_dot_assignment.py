#!/usr/bin/env python3
"""GAS location-counter assignment and `.org` parity for the x86 assembler.

Covers the Linux kexec fixed-width vector pattern (`. = base + (n * WIDTH)`),
macro expansion, numeric local labels, dot-relative advances, relaxation-driven
padding fixup, fill bytes, and fail-closed rejection of malformed or
unsupported expressions. Loaded bytes and symbol values are compared against
the pinned GNU assembler whenever a form is representable by both.
"""

from __future__ import annotations

import argparse
import pathlib
import shutil
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[2]

VECTOR_SRC = r"""
.set WIDTH, 6
.text
.globl base
base:
.macro vec n
. = base + (\n * WIDTH)
nop
nop
pushq $\n
jmp target
.endm
vec 0
vec 1
vec 2
target:
ret
.globl end
end:
.data
.globl data_base
data_base:
. = data_base + (3 * 4)
.byte 7
"""

LOCAL_SRC = ".text\n2:\nnop\n. = 2b + 8\nnop\n"
DOT_REL_SRC = ".text\nnop\n. = . + 4\nret\n"
ABS_SRC = ".text\n. = 8\nret\n"
RELAX_SRC = ".text\nbase:\nnop\njmp target\n. = base + 8\ntarget:\nret\n"
MULTI_PLUS_SRC = ".text\nbase:\n. = base + 4 + 4\nret\n"
PAREN_SRC = ".text\nbase:\n. = (base + 8)\nret\n"
ORG_FILL_SRC = ".text\nbase:\nnop\n.org base + 12, 0x77\nret\n"
ORG_DOT_SRC = ".text\nnop\n.org . + 4, 0x90\nret\n"

NEGATIVE_SRCS = {
    "backwards": ".text\n. = 8\n. = 4\nret\n",
    "unbalanced": ".text\n. = (1 + 2\nret\n",
    "unsupported_product": ".text\n. = unknown * 2\nret\n",
    "double_equal": ".text\n. == 8\nret\n",
    "cross_section": (
        ".text\nnop\n.data\nbase:\n.byte 1\n.text\n. = base + 8\nret\n"
    ),
    "symbol_minus_backwards": ".text\nbase:\nnop\n. = base - 1\nret\n",
}


def run(cmd: list[object], expect: int = 0) -> subprocess.CompletedProcess[str]:
    argv = [str(part) for part in cmd]
    result = subprocess.run(argv, capture_output=True, text=True)
    if result.returncode != expect:
        raise AssertionError(
            f"command exited {result.returncode}, expected {expect}: {argv}\n"
            f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
        )
    return result


def section_bytes(objcopy: str, obj: pathlib.Path, section: str,
                  dest: pathlib.Path) -> bytes:
    run([objcopy, f"--only-section={section}", "-O", "binary", obj, dest])
    return dest.read_bytes()


def symbols(nm: str, obj: pathlib.Path, names: set[str]) -> dict[str, int]:
    result = run([nm, "-n", obj])
    found: dict[str, int] = {}
    for line in result.stdout.splitlines():
        parts = line.split()
        if len(parts) >= 3 and parts[-1] in names:
            found[parts[-1]] = int(parts[0], 16)
    return found


def compare_positive(tmp: pathlib.Path, name: str, src_text: str, bits: int,
                     lccc: pathlib.Path, gas: str, objcopy: str, nm: str,
                     sections: tuple[str, ...] = (".text",),
                     syms: set[str] = frozenset()) -> None:
    src = tmp / f"{name}-{bits}.s"
    src.write_text(src_text)
    lccc_obj = tmp / f"{name}-{bits}-lccc.o"
    gas_obj = tmp / f"{name}-{bits}-gas.o"
    run([lccc, f"-m{bits}", "-c", src, "-o", lccc_obj])
    run([gas, f"--{bits}", "-o", gas_obj, src])
    for section in sections:
        a = section_bytes(objcopy, lccc_obj, section, tmp / "lccc.bin")
        b = section_bytes(objcopy, gas_obj, section, tmp / "gas.bin")
        assert a == b, f"{name} ELF{bits} {section} bytes differ: {a.hex()} != {b.hex()}"
    if syms:
        assert symbols(nm, lccc_obj, syms) == symbols(nm, gas_obj, syms), (
            f"{name} ELF{bits} symbols differ"
        )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lccc", type=pathlib.Path,
                        default=ROOT / "target/fastbuild/lccc")
    parser.add_argument("--gas", default="as")
    args = parser.parse_args()
    lccc = args.lccc.resolve()
    gas = shutil.which(args.gas)
    objcopy = shutil.which("objcopy")
    nm = shutil.which("nm")
    if not lccc.is_file():
        print(f"FAIL: build lccc first ({lccc})", file=sys.stderr)
        return 1
    if not gas or not objcopy or not nm:
        print("FAIL: GNU as, objcopy and nm are required for the parity oracle",
              file=sys.stderr)
        return 1

    with tempfile.TemporaryDirectory(prefix="lccc-dot-") as td:
        tmp = pathlib.Path(td)
        for bits in (64, 32):
            vector = VECTOR_SRC if bits == 64 else VECTOR_SRC.replace("pushq", "pushl")
            compare_positive(tmp, "vector", vector, bits, lccc, gas, objcopy, nm,
                             sections=(".text", ".data"),
                             syms={"base", "end", "data_base"})
            # The vector table is exactly three 6-byte entries plus the ret.
            vec_bin = section_bytes(objcopy, tmp / f"vector-{bits}-lccc.o",
                                    ".text", tmp / "vec.bin")
            assert vec_bin[:18] == bytes.fromhex(
                "90906a00eb0c90906a01eb0690906a02eb00"
            ), vec_bin[:18].hex()
            assert len(vec_bin) == 19, len(vec_bin)

            for name, src_text in (
                ("local", LOCAL_SRC),
                ("dotrel", DOT_REL_SRC),
                ("abs", ABS_SRC),
                ("relax", RELAX_SRC),
                ("multiplus", MULTI_PLUS_SRC),
                ("paren", PAREN_SRC),
                ("orgfill", ORG_FILL_SRC),
                ("orgdot", ORG_DOT_SRC),
            ):
                compare_positive(tmp, name, src_text, bits, lccc, gas, objcopy, nm)

            for name, src_text in NEGATIVE_SRCS.items():
                src = tmp / f"neg-{name}-{bits}.s"
                src.write_text(src_text)
                result = run([lccc, f"-m{bits}", "-c", src,
                              "-o", tmp / "neg.o"], expect=1)
                assert result.stderr.strip(), f"{name} failed without a diagnostic"

            print(f"PASS ELF{bits}: macro vectors, locals, dot-relative, relaxation, "
                  "fill, GAS bytes/symbols, fail-closed negatives")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
