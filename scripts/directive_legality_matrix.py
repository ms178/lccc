#!/usr/bin/env python3
"""The directive-legality matrix: what GNU as says about .p2align/.fill/.rept.

WHY THIS EXISTS
---------------
The operand-legality matrix (``aarch64_operand_legality_matrix.py``) pins
*instruction* verdicts; the dispatcher goldens pin *encodings*.  Neither
reaches *directive* verdicts, and the PR768 review showed why that gap
matters: its H1 claim (".text rejects a capped `.p2align 63` skip") was
measured against binutils 2.40 and did not reproduce on the pinned
oracles, while the unit tests that should have contradicted it all used
``.data`` only.  This is the instrument for that class: a curated matrix
of directive inputs — valid and deliberately pathological — whose
expected verdicts are re-derived from the pinned GAS 2.47 pair on every
pass, with SECTION-KIND coverage (.text / .data / custom ax / custom w)
and both targets (aarch64 and x86_64), then asserted against our own
front ends.

EXPECTATIONS COME FROM THE ORACLE, NEVER FROM US
-------------------------------------------------
``--regenerate`` rewrites the table from GAS alone; ``--check`` re-runs
every row against GAS and fails on drift (keeps the table from rotting
when binutils changes its mind); ``--check-lccc`` runs the same rows
against our driver (``-c``, backend selected through an argv[0] symlink,
the same mechanism the operand matrix uses).  One direction alone proves
nothing: a table can be perfect while the implementation disagrees with
every row of it.

WHICH ROWS ARE *NOT* HERE
-------------------------
Uncapped absurd pads in data sections and on aarch64 are free-state on
the oracle itself (GAS tries to materialize 2^63 bytes and dies however
the machine happens to be configured — ENOSPC, OOM-kill, or timeout), so
their verdict is not machine-invariant and must not be pinned here.
Those cases are pinned as deterministic-stand-in verdicts (always Err)
in the ``gas247_absurd_*`` unit tests instead; this table carries only
rows whose oracle verdict is identical on every machine: positive-cap
skips (accept), offset-0 no-ops (accept), x86-64 executable-section
uncapped pads (the semantic "jump over nop padding out of range"), the
fill wrap laws, and the `do_repeat` budget errors (which fire before any
allocation).

USAGE
-----
    scripts/directive_legality_matrix.py --regenerate          # rewrite table from GAS
    scripts/directive_legality_matrix.py --check               # table == GAS
    scripts/directive_legality_matrix.py --check-lccc target/fastbuild/lccc

Exit codes mirror the operand matrix: 0 match, 1 mismatch, 2 missing
tools (so a caller can tell "wrong" from "cannot tell").
"""
from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
TABLE = REPO / "tests" / "directive-legality.tsv"

GAS_A64 = Path.home() / ".cache/gas-2.47-aarch64-linux-gnu/bin/as"
GAS_X64 = Path.home() / ".cache/gas-2.47-x86_64-linux-gnu/bin/as"

# arch -> (expected section names in emitted object, assembler binary)
ARCHS = {
    "a64": {"as": GAS_A64, "triple": "aarch64-linux-gnu"},
    "x64": {"as": GAS_X64, "triple": "x86_64-linux-gnu"},
}

# Grouped by the rule each row answers, so a failure names the law that
# broke rather than a source file.  Sources use literal "\n" escapes
# (the table stays one row per line, three-tab fields like the operand
# matrix).
P2ALIGN_ACCEPT = [
    # Positive max-skip skip at offset 1 — section-kind axis (H1): the
    # skip verdict is identical in .text, .data and custom ax/w sections
    # on both pinned targets.
    '.text\\n.byte 1\\n.p2align 63,,5\\n.byte 2\\n',
    '.data\\n.byte 1\\n.p2align 63,,5\\n.byte 2\\n',
    '.section .cax,"ax"\\n.byte 1\\n.p2align 63,,5\\n.byte 2\\n',
    '.section .cw,"w"\\n.byte 1\\n.p2align 63,,5\\n.byte 2\\n',
    '.text\\n.byte 1\\n.p2align 63,,1\\n.byte 2\\n',
    '.text\\n.byte 1\\n.p2align 63,,100\\n.byte 2\\n',
    '.data\\n.byte 1\\n.p2align 63,,1\\n.byte 2\\n',
    # Offset 0: no-op, no raise, any cap, any section.
    '.text\\n.p2align 63,,5\\n.byte 3\\n',
    '.text\\n.p2align 63\\n.byte 3\\n',
    '.data\\n.p2align 63\\n.byte 3\\n',
    # Representable alignments: unlimited max-skip pads fully; a huge
    # but representable exponent skips AND raises sh_addralign.
    '.data\\n.byte 1\\n.p2align 4,,0\\n.byte 2\\n',
    '.text\\n.byte 1\\n.p2align 62,,5\\n.byte 2\\n',
]

# x86-64 executable sections give the deterministic semantic error for
# uncapped/l unlimited pads (tc-i386 "jump over nop padding out of range").
# The aarch64/data uncapped cases are free-state on the oracle and are
# therefore NOT row-pinned (see docstring); they live in the unit tests.
P2ALIGN_X64_REJECT = [
    '.text\\n.byte 1\\n.p2align 63\\n.byte 2\\n',
    '.text\\n.byte 1\\n.p2align 63,,0\\n.byte 2\\n',
    '.section .cax,"ax"\\n.byte 1\\n.p2align 63,,0\\n.byte 2\\n',
]

FILL = [
    # 2^62 * 4 wraps mod 2^64 to zero bytes: GAS 2.47 and llvm-mc 23.1.2
    # both accept with sh_size 0 (PR768 M1 adjudication).
    '.data\\n.fill 4611686018427387904,4\\n',
    # Element size clamps to 8 (BSD 4.2 crock): 1048576 * 8 = 8 MiB.
    '.data\\n.fill 1048576,17\\n',
    # Non-positive repeat/size -> 0 bytes; small fill stays literal.
    '.data\\n.fill -1,4\\n',
    '.data\\n.fill 0,4\\n',
    '.data\\n.fill 5,9\\n',
    '.data\\n.fill 4,1,255\\n',
]

REPT = [
    '.data\\n.rept 2\\n.byte 7\\n.endr\\n',
    '.data\\n.rept 0\\n.byte 7\\n.endr\\n',
    # do_repeat budget (read.c): negative wraps to size_t, and
    # count * effective_body_len > 0xffffffff is a hard error — all
    # three fire BEFORE any allocation, so these verdicts are stable.
    '.data\\n.rept -1\\n.byte 7\\n.endr\\n',
    '.data\\n.rept 613566757\\n.byte 0\\n.endr\\n',
    '.data\\n.rept 1099511627776\\n.byte 0\\n.endr\\n',
    # Shift-count warning + zero count on both sides (GAS warns and
    # expands nothing; we match the rc/size verdict).
    '.data\\n.rept 1<<64\\n.byte 0\\n.endr\\n',
    # Zero-divisor law (measured): GAS warns and yields LHS/0, so
    # `.rept 7/0` expands 7 times and `.rept 7%0` expands zero times,
    # both rc 0 -- hard-errors here rejected legal GAS input.
    '.data\\n.rept 7/0\\n.byte 9\\n.endr\\n',
    '.data\\n.rept 7%0\\n.byte 9\\n.endr\\n',
]

def _macro_chain(n: int) -> str:
    """`n` distinct macros chained m0 -> m1 -> ... -> m{n-1} -> `.byte 7`.

    The macro-nesting wall, re-measured on the pinned GAS 2.47 for both
    targets: a 101-frame chain assembles, the 102nd frame fatals with
    `macros nested too deeply`, and direct/mutual recursion land on the
    same wall — which is also the verdict we used to *crash* on (native
    stack overflow) before the frame counter existed.
    """
    parts = [".data"]
    for i in range(n):
        nxt = f"m{i + 1}" if i + 1 < n else ".byte 7"
        parts.append(f".macro m{i}\\n{nxt}\\n.endm")
    parts.append("m0")
    return "\\n".join(parts) + "\\n"


# Direct and mutual recursion: both must be REJECT, never a crash (the
# old x86 path died with SIGABRT/stack overflow on these two rows).
MACRO_SELFREC = ".data\\n.macro m\\n m\\n.endm\\n m\\n"
MACRO_MUTUAL = ".data\\n.macro a\\n b\\n.endm\\n.macro b\\n a\\n.endm\\n a\\n"
# Sequential invocations must NOT accumulate frames: GAS assembles any
# number of top-level calls, so depth is released when a body returns.
MACRO_SEQUENTIAL = ".data\\n.macro n\\n.byte 7\\n.endm\\n" + "n\\n" * 500

MACRO = [
    _macro_chain(101),
    _macro_chain(102),
    MACRO_SELFREC,
    MACRO_MUTUAL,
    MACRO_SEQUENTIAL,
]

GROUPS: dict[str, list[str]] = {
    "p2align": P2ALIGN_ACCEPT,
    "p2align-x64": P2ALIGN_X64_REJECT,
    "fill": FILL,
    "rept": REPT,
    "macro": MACRO,
}


def rows():
    """(arch, group, source, _) for every applicable row."""
    for group, srcs in GROUPS.items():
        for src in srcs:
            for arch in ("a64", "x64"):
                if group == "p2align-x64" and arch != "x64":
                    continue
                yield arch, group, src, ""


def section_of(src: str) -> str:
    """The section the row dirties (first section directive, else .text)."""
    for line in src.replace("\\n", "\n").splitlines():
        line = line.strip()
        if line.startswith(".section "):
            return line.split()[1].split(",")[0]
        if line.startswith(".") and line[1:2].isalpha() and line.split()[0] in (
            ".text", ".data", ".rodata", ".bss",
        ):
            return line.split()[0]
    return ".text"


def parse_section(o: Path, name: str) -> tuple[int, int]:
    """(sh_size, sh_addralign) of `name` from an ELF64 object."""
    r = subprocess.run(["readelf", "-SW", str(o)], capture_output=True, text=True)
    if r.returncode != 0:
        raise RuntimeError(f"readelf failed: {r.stderr[:200]}")
    for line in r.stdout.splitlines():
        m = re.search(r"\[\s*\d+\]\s+(\S+)\s+(PROGBITS|NOBITS)\b", line)
        if not m or m.group(1) != name:
            continue
        toks = line.split()
        i = toks.index(m.group(1))
        size = int(toks[i + 4], 16)
        align = int(toks[-1])
        return size, align
    raise RuntimeError(f"section {name} not found in {o.name}")


def _prep(tmp: Path, src: str) -> tuple[Path, Path]:
    """Write the row's .s file (\n-unescaped) and return (s, o)."""
    s, o = tmp / "row.s", tmp / "row.o"
    s.write_text(src.replace("\\n", "\n"))
    if o.exists():
        o.unlink()
    return s, o


def _verdict(src: str, rc: int, o: Path) -> str:
    """Map an assembler run to a verdict string.

    Exit 0 = accept, exit 1 = reject -- and NOTHING else is a verdict.
    A signal death (>= 128: SIGSEGV/SIGABRT/OOM-kill) or a Rust panic
    (101) is a harness failure, not a legal answer: classifying any
    nonzero exit as REJECT would let a crash silently satisfy every
    REJECT row (the operand matrix pinned this law first; a regression
    that brought back the 2^62 `Vec::with_capacity` abort must never
    read as "assembler said no").
    """
    if rc == 1:
        return "REJECT"
    if rc != 0:
        raise RuntimeError(f"assembler died (rc={rc}), not a verdict")
    size, align = parse_section(o, section_of(src))
    return f"ACCEPT|{section_of(src)}|{size}|{align}"


def run_gas(arch: str, src: str, tmp: Path) -> str:
    """Verdict string for one row against the pinned GAS."""
    as_bin = ARCHS[arch]["as"]
    if not as_bin.is_file():
        raise FileNotFoundError(str(as_bin))
    s, o = _prep(tmp, src)
    r = subprocess.run([str(as_bin), "-o", str(o), str(s)],
                       capture_output=True, text=True, timeout=30)
    return _verdict(src, r.returncode, o)


def run_lccc(lccc: Path, arch: str, src: str, tmp: Path) -> str:
    """Verdict string for one row against our driver."""
    s, o = _prep(tmp, src)
    triple = ARCHS[arch]["triple"]
    cc = tmp / f"{triple}-ccc"
    if not cc.is_symlink():
        cc.symlink_to(lccc.resolve())
    r = subprocess.run([str(cc), "-c", str(s), "-o", str(o)],
                       capture_output=True, text=True, timeout=30)
    return _verdict(src, r.returncode, o)


def load_table(path: Path) -> list[tuple[str, str, str, str]]:
    out = []
    for line in path.read_text().splitlines():
        if not line or line.startswith("#"):
            continue
        parts = line.split("\t")
        if len(parts) != 4:
            raise ValueError(f"bad row (want 4 tab fields): {line!r}")
        out.append(tuple(parts))  # type: ignore[arg-type]
    return out


def write_table(path: Path, rows_: list[tuple[str, str, str, str]]) -> None:
    header = (
        "# Directive-legality matrix -- expectations come from GNU as 2.47.\n"
        "# Generated by scripts/directive_legality_matrix.py; do not hand-edit.\n"
        "# Every row is re-checked against GAS by --check (the\n"
        "# directive-legality gate in ci_local.sh) and against our own front\n"
        "# ends by --check-lccc (directive-legality-lccc).  Free-state oracle\n"
        "# verdicts are deliberately absent -- see the script docstring.\n"
        "# Columns: arch, group, source (\\n-escaped), expected verdict\n"
        "#   expected = REJECT  |  ACCEPT|section|sh_size|sh_addralign\n"
    )
    body = "".join(f"{a}\t{g}\t{s}\t{e}\n" for a, g, s, e in rows_)
    tmp = path.with_suffix(".tsv.tmp")
    tmp.write_text(header + body)
    tmp.replace(path)


def tool_missing(msg: str) -> int:
    print(f"MISSING TOOLS: {msg}", file=sys.stderr)
    return 2


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    mode = ap.add_mutually_exclusive_group(required=True)
    mode.add_argument("--regenerate", action="store_true")
    mode.add_argument("--check", action="store_true")
    mode.add_argument("--check-lccc", metavar="PATH")
    ap.add_argument("--gas-a64", metavar="PATH", default=None,
                    help="pinned aarch64 `as` (default: the ensure_gas_247.sh "
                         "cache); spelled literally by the CI gates")
    ap.add_argument("--gas-x64", metavar="PATH", default=None,
                    help="pinned x86_64 `as` (default: the ensure_gas_247.sh "
                         "cache); spelled literally by the CI gates")
    args = ap.parse_args()
    if args.gas_a64:
        ARCHS["a64"]["as"] = Path(args.gas_a64)
    if args.gas_x64:
        ARCHS["x64"]["as"] = Path(args.gas_x64)

    tmp = Path(tempfile.mkdtemp(prefix="dirmatrix."))
    try:
        if args.regenerate:
            out = []
            for arch, group, src, _ in rows():
                try:
                    verdict = run_gas(arch, src, tmp)
                except FileNotFoundError as e:
                    return tool_missing(str(e))
                except (subprocess.TimeoutExpired, RuntimeError) as e:
                    print(f"ORACLE ERROR {arch} {group} {src!r}: {e}",
                          file=sys.stderr)
                    return 1
                out.append((arch, group, src, verdict))
            write_table(TABLE, out)
            print(f"OK: regenerated {len(out)} rows from GAS")
            return 0

        if args.check:
            try:
                table = load_table(TABLE)
            except FileNotFoundError:
                return tool_missing(f"table {TABLE} missing (run --regenerate)")
            bad = 0
            for arch, group, src, expect in table:
                try:
                    got = run_gas(arch, src, tmp)
                except FileNotFoundError as e:
                    return tool_missing(str(e))
                except (subprocess.TimeoutExpired, RuntimeError) as e:
                    print(f"MISMATCH {arch} {group} {src!r}: oracle error {e}")
                    bad += 1
                    continue
                if got != expect:
                    print(f"MISMATCH {arch} {group} {src!r}: table={expect!r} gas={got!r}")
                    bad += 1
            if bad:
                print(f"FAIL: {bad}/{len(table)} rows disagree with GAS")
                return 1
            print(f"OK: {len(table)} rows match GNU as (2.47, both targets)")
            return 0

        # --check-lccc
        lccc = Path(args.check_lccc)
        if not lccc.is_file():
            return tool_missing(f"lccc binary {lccc} not found")
        try:
            table = load_table(TABLE)
        except FileNotFoundError:
            return tool_missing(f"table {TABLE} missing (run --regenerate)")
        bad = 0
        for arch, group, src, expect in table:
            try:
                got = run_lccc(lccc, arch, src, tmp)
            except (FileNotFoundError, subprocess.TimeoutExpired, RuntimeError) as e:
                print(f"MISMATCH {arch} {group} {src!r}: lccc error {e}")
                bad += 1
                continue
            if got != expect:
                print(f"MISMATCH {arch} {group} {src!r}: table={expect!r} lccc={got!r}")
                bad += 1
        if bad:
            print(f"FAIL: {bad}/{len(table)} rows disagree with lccc")
            return 1
        print(f"OK: {len(table)} rows match lccc (both targets)")
        return 0
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
