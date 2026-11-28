#!/usr/bin/env python3
"""How long is the code for ONE call of each x86 intrinsic, against GCC's?

Why
---
`x86_intrinsic_census.py` says which intrinsics a program can call under
lccc and `x86_intrinsic_values.py` that a call computes what GCC's does.
Neither says what the call costs.  An intrinsic is supposed to be an
instruction; where lccc's header writes it as a loop over the lanes, or the
operands take a detour through the stack, it is twenty or a hundred, and no
test fails.  This tool measures that for every intrinsic at once.

How
---
For every intrinsic of GCC's headers that both compilers take for the
target, the caller the census writes (the header's own types as parameters,
a constant where the macro twin wants one)

    __m256i t7(__m256i a0, __m256i a1) { return _mm256_adds_epi8(a0, a1); }

is compiled by both at -O2, and the instructions of each function are
counted (`ret` included; directives, labels and comments are not).  The
report is a worklist: the callers lccc needs the most instructions more
for, each with the marks that name a cause -- `S` the function touches the
stack, `A` it aligns an address at run time (`andq $-32`/`$-64`: an
over-aligned home).

The count is of instructions, not of time: it finds lane loops, homes,
copies and reloads; it does not rank a `vpermd` against a `vpshufd`.  And a
caller is the worst case for the calling convention (every operand arrives
as a parameter and leaves as the result), which is the point: what is paid
there is paid wherever such a value crosses a function.

    scripts/x86_intrinsic_shapes.py --lccc target/fastbuild/lccc --only avx2intrin.h
    scripts/x86_intrinsic_shapes.py --lccc target/fastbuild/lccc --out shapes.tsv

The default target is `-march=x86-64-v3` (AVX2, FMA, BMI), the same on every
machine; `--flags "-march=native"` measures the host's.  `--out` writes one
line per intrinsic (difference, lccc, GCC, marks, header, name), which two
runs can be compared by.

GCC must be 16.2 (`--gcc`, `GCC`, default `gcc-16`): the headers are the
list.
"""
import argparse
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from scripts import x86_intrinsic_census as census  # noqa: E402

LABEL = re.compile(r"([A-Za-z_][A-Za-z0-9_]*):")
STACK = re.compile(r"\(%r[sb]p[,)]|^(?:push|pop)[qlw]?\s")
ALIGN = re.compile(r"^andq \$-(?:32|64), ")


def functions(assembly: str) -> dict[str, list[str]]:
    out: dict[str, list[str]] = {}
    current = None
    for raw in assembly.splitlines():
        line = raw.split("#")[0].strip()
        if not line:
            continue
        label = LABEL.fullmatch(line)
        if label:
            current = label.group(1)
            out[current] = []
        elif line.startswith(".") or line.endswith(":"):
            continue
        elif current is not None:
            out[current].append(re.sub(r"\s+", " ", line))
    return out


def assembly_of(command: list[str], lines: list[str], tmp: Path) -> dict[str, list[str]]:
    source = tmp / "s.c"
    source.write_text(census.HEAD + "\n".join(lines) + "\n")
    result = subprocess.run([*command, "-S", "-o", "-", str(source)], capture_output=True, text=True)
    if result.returncode != 0:
        raise RuntimeError(f"{command[0]} refuses what it took line by line:\n{result.stderr[:2000]}")
    return functions(result.stdout)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--lccc", default=str(ROOT / "target" / "release" / "lccc"))
    parser.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"),
                        help="GCC 16.2: its headers are the list")
    parser.add_argument("--only", metavar="HEADER", action="append", default=[])
    parser.add_argument("--flags", default="-march=x86-64-v3",
                        help="the target both compilers compile the callers for")
    parser.add_argument("--top", type=int, default=40,
                        help="how many of the longest callers to print")
    parser.add_argument("--out", default="", metavar="FILE",
                        help="one line per intrinsic, tab separated")
    args = parser.parse_args()
    if not Path(args.lccc).exists():
        print(f"x86_intrinsic_shapes: no lccc at {args.lccc} (--lccc)", file=sys.stderr)
        return 2
    args.lccc = str(Path(args.lccc).resolve())
    version = census.gcc_version_of(args.gcc)
    if version != census.GCC_VERSION:
        print(f"x86_intrinsic_shapes: the headers are GCC {census.GCC_VERSION}'s; {args.gcc} is "
              f"{version or 'not runnable'} (--gcc, or GCC=)", file=sys.stderr)
        return 2
    flags = args.flags.split()
    include = Path(subprocess.run([args.gcc, "-print-file-name=include"], capture_output=True,
                                  text=True).stdout.strip())
    rows = []
    lacking = 0
    with tempfile.TemporaryDirectory() as directory:
        tmp = Path(directory)
        for header in census.headers_of(include):
            if args.only and header.name not in args.only:
                continue
            found = census.functions_of(header.read_text(errors="replace"))
            names = sorted(found)
            if not names:
                continue
            lines = [census.caller(i, name, *found[name]) for i, name in enumerate(names)]
            skipped = census.refused([args.gcc, *flags, "-O2"], lines, tmp, census.OBJECT)
            kept = [i for i in range(len(lines)) if i not in skipped]
            bad = census.refused([args.lccc, *flags, "-O2"], [lines[i] for i in kept], tmp,
                                 census.OBJECT)
            lacking += len(bad)
            both = [kept[i] for i in range(len(kept)) if i not in bad]
            if not both:
                continue
            ours = assembly_of([args.lccc, *flags, "-O2"], [lines[i] for i in both], tmp)
            theirs = assembly_of([args.gcc, *flags, "-O2"], [lines[i] for i in both], tmp)
            for i in both:
                mine, reference = ours.get(f"t{i}"), theirs.get(f"t{i}")
                if mine is None or reference is None:
                    continue
                marks = ("S" if any(STACK.search(line) for line in mine) else "") + (
                    "A" if any(ALIGN.match(line) for line in mine) else "")
                rows.append((len(mine) - len(reference), len(mine), len(reference), marks,
                             header.name, names[i]))
    rows.sort(key=lambda row: (-row[0], row[5]))
    total_ours = sum(row[1] for row in rows)
    total_theirs = sum(row[2] for row in rows)
    longer = sum(1 for row in rows if row[0] > 0)
    summary = (f"{len(rows)} intrinsics compared ({lacking} more lccc lacks for this target);"
               f" lccc {total_ours} instructions, GCC {total_theirs};"
               f" {longer} longer than GCC's, {sum(1 for r in rows if 'A' in r[3])} align an address,"
               f" {sum(1 for r in rows if 'S' in r[3])} touch the stack")
    print(summary)
    for delta, mine, reference, marks, header, name in rows[:args.top]:
        print(f"{delta:+4d}  {mine:3d} vs {reference:3d}  {marks:2s}  {name}  ({header})")
    if args.out:
        with open(args.out, "w", encoding="utf-8") as handle:
            handle.write(summary + "\n")
            for delta, mine, reference, marks, header, name in rows:
                handle.write(f"{delta:+d}\t{mine}\t{reference}\t{marks}\t{header}\t{name}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
