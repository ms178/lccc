#!/usr/bin/env python3
"""Which x86 intrinsics of GCC 16.2 can a program call under lccc?

Why
---
`check_x86_march_macros.py` and `check_x86_mflag_macros.py` hold the
*macros* lccc defines to GCC's: `__AVX512F__` under `-march=skylake-avx512`,
`__F16C__` under `-mf16c`.  A macro is a promise: code behind `#ifdef
__AVX512F__` calls `_mm512_mask_loadu_ps`, and if lccc has no such
intrinsic the program that GCC compiles does not compile here.  Nothing
measured that promise.  This tool does, against the reference's own list.

How
---
Every inline function of GCC's x86 headers (`*intrin.h`, `mm3dnow.h`,
`mm_malloc.h`) is read with its return and parameter types, and a caller is
written for it:

    __m512i t7(__m512i a0, __mmask16 a1, __m512i a2, __m512i a3)
    { return _mm512_mask_add_epi32(a0, a1, a2, a3); }

A parameter the header's function-like macro twin takes as an immediate
gets a constant (8, a valid rounding mode, where the header calls it a
rounding; 1 otherwise).  The callers of one header are compiled together.
GCC goes first, with every extension enabled and `-fsyntax-only`: the
question to GCC is what its headers let a program call, and a caller GCC
itself refuses is this tool's mistake, not a finding, and is dropped
(`skipped`).  Then lccc, which has to make an object file of the callers:
the ones it reports an error in are `missing`, and are taken out until the
rest compiles.  (lccc's `-fsyntax-only` stops in front of the lowering,
where a wrapper whose builtin does not exist, or whose instruction the
target lacks, is refused: a declaration is not yet a call that compiles.)

The reader is held to GCC's own list.  GCC is asked what the headers
define (`-aux-info`, with and without `__OPTIMIZE__`, which chooses between
a function and its macro twin), and a definition GCC compiled that the
reader did not find is an error of this tool, reported and fatal: the
headers write a definition in more ways than a pattern foresees (the name
after one space, two or none; indented; `__inline` or `__inline__`; a
blank line in front of the body), and three patterns in a row each read a
census that was short, by 148, 3 and 292 of 7,500 definitions.

`cpuid.h` is left out: lccc has no such header and reads the system
compiler's, so its four functions say nothing about lccc.

The verdict is about *availability*: a caller that compiles may still be
wrong code.  That is the business of `x86_intrinsic_values.py`, which runs
what this tool counts; this is the inventory an execution suite cannot
give, because a test nobody wrote fails nowhere.

Record
------
`tests/x86-intrinsics/expected.txt` holds, per header, how many intrinsics
GCC lets a program call, how many of them lccc lacks, and a digest of their
names.  `--check` fails on any difference, in either direction: a lost
intrinsic is a regression, and a gained one is a record that has to say so.
`--update` rewrites it; `--list HEADER` prints the missing names of one
header, which is the worklist.

    scripts/x86_intrinsic_census.py --lccc target/fastbuild/lccc --check
    scripts/x86_intrinsic_census.py --lccc target/fastbuild/lccc --list f16cintrin.h

GCC must be 16.2 (`--gcc`, `GCC`, default `gcc-16`): the headers are the
list.
"""
from __future__ import annotations

import argparse
import hashlib
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
EXPECTED = ROOT / "tests" / "x86-intrinsics" / "expected.txt"
GCC_VERSION = "16.2.0"

# Every extension GCC 16.2 has intrinsics for, so that no caller is refused
# for its target: the newest CPU and what no CPU row has.
GCC_FLAGS = (
    "-march=diamondrapids -mamx-tile -mamx-int8 -mamx-bf16 -mamx-fp16 -mamx-complex "
    "-mamx-avx512 -mamx-fp8 -mamx-movrs -mamx-tf32 -mfma4 -mxop -mlwp -mtbm -msse4a "
    "-mkl -mwidekl -mraoint -muintr -mhreset -mprefetchi -mclzero -mmwaitx -mrtm -msgx "
    "-mavx512vp2intersect -mavx512bmm -mptwrite -mpconfig -mwbnoinvd -mcldemote -menqcmd "
    "-mserialize -mtsxldtrk -mwaitpkg -mmovdiri -mmovdir64b -mpku -mshstk -musermsr "
    "-mxsavec -mxsaves -mxsaveopt -mfsgsbase -mrdpid -mrdseed -madx -mclwb -mclflushopt "
    "-mmwait"
).split()

# The widest target lccc accepts: its newest CPU and every extension it
# takes as an option that the CPU does not have.
LCCC_FLAGS = (
    "-march=diamondrapids -msha512 -mavxvnniint8 -mavxvnniint16 -mavxifma -mavxneconvert "
    "-mcmpccxadd -msm3 -msm4 -mmovrs -mamx-tile -mamx-int8 -mamx-bf16 -musermsr"
).split()

HEAD = "#include <immintrin.h>\n#include <x86intrin.h>\n"
# A whole compilation, the object file left in the working directory.
OBJECT = ("-c", "-o", "census.o")

# The x86 headers that are not named `*intrin.h`.
OTHER_HEADERS = ("mm3dnow.h", "mm_malloc.h")

# A definition is found by its name: an identifier that starts a line, its
# parameter list, and the brace of the body.  How GCC's headers write the
# rest varies (`_mm_add_ps (`, `_mm_shuffle_pd(`, `_mm256_set_epi8  (`; the
# return type beside `extern __inline` or on the next line; the attributes
# in front of it or behind; the whole definition indented), so nothing else
# is asked of the layout, and `census` holds the result to what GCC
# compiled (`compiled_definitions`).
DEFINED_NAME = re.compile(r"^[ \t]*(_[A-Za-z0-9_]+)[ \t]*\(([^)]*)\)\s*\{", re.M)
# Its declaration specifiers stand between this and the name.
INLINE = re.compile(r"^[ \t]*(?:extern|static)\s+(?:__inline__|__inline|inline)\b", re.M)
FUNCTION_MACRO = re.compile(r"^#\s*define\s+(_[A-Za-z0-9_]+)\(", re.M)
# A parameter: its type, its name, and the brackets of an array.
PARAMETER = re.compile(r"^(.*?)\s*\b(__[A-Za-z0-9_]+)\s*((?:\[[^\]]*\]\s*)*)$")

# The types an immediate has in the headers.
IMMEDIATE_TYPES = {
    "int", "unsigned int", "unsigned", "unsigned char", "char", "_MM_PERM_ENUM",
    "_MM_MANTISSA_NORM_ENUM", "_MM_MANTISSA_SIGN_ENUM", "_MM_CMPINT_ENUM", "enum _mm_hint",
}


def without_attributes(specifiers: str) -> str:
    """`specifiers` without its `__attribute__ ((...))`, parentheses matched."""
    out, at = [], 0
    while True:
        found = specifiers.find("__attribute__", at)
        if found < 0:
            return "".join(out) + specifiers[at:]
        out.append(specifiers[at:found])
        at = found + len("__attribute__")
        while at < len(specifiers) and specifiers[at].isspace():
            at += 1
        depth = 0
        while at < len(specifiers):
            depth += {"(": 1, ")": -1}.get(specifiers[at], 0)
            at += 1
            if depth == 0:
                break


def parameter_of(parameter: str) -> tuple[str, str]:
    """(type, name) of one parameter; an array is the pointer it decays to."""
    match = PARAMETER.match(parameter)
    if not match:
        return parameter, ""
    kind, name, array = match.group(1).strip(), match.group(2), match.group(3)
    return (f"{kind} *" if array.strip() else kind), name


def functions_of(text: str) -> dict[str, tuple[str, list[str], bool]]:
    """name -> (return type, parameters, has a function-like macro twin)."""
    twins = set(FUNCTION_MACRO.findall(text))
    inlines = [match.end() for match in INLINE.finditer(text)]
    found: dict[str, tuple[str, list[str], bool]] = {}
    position = 0
    for match in DEFINED_NAME.finditer(text):
        name, parameters = match.group(1), match.group(2).strip()
        # The last `extern __inline` in front of the name, with nothing but
        # declaration specifiers in between.
        while position + 1 < len(inlines) and inlines[position + 1] <= match.start():
            position += 1
        if not inlines or inlines[position] > match.start():
            continue
        specifiers = text[inlines[position]:match.start()]
        if re.search(r"[;{}#]", specifiers):
            continue
        returns = " ".join(without_attributes(specifiers).split())
        if name in found or name.startswith("__builtin") or not returns:
            continue
        listed = [] if parameters in ("", "void") else [
            " ".join(p.split()) for p in re.split(r",(?![^(]*\))", parameters)]
        found[name] = (returns, listed, name in twins)
    return found


AUX_DEFINITION = re.compile(r"^/\* (\S+):\d+:[A-Z]F \*/ (.*?); /\*", re.M)


def aux_definitions_of(text: str, include: Path) -> dict[str, set[str]]:
    """header -> the functions it defines, from a `gcc -aux-info` file.

    A line is `/* FILE:LINE:NF */ extern int __bsfd (int __X); /* ... */`:
    `F` says definition (`C` is a declaration), and the name is the
    identifier in front of the parameter list."""
    found: dict[str, set[str]] = {}
    root = os.path.realpath(include)
    for path, declaration in AUX_DEFINITION.findall(text):
        path = os.path.realpath(path)
        name = re.search(r"\b(_[A-Za-z0-9_]+) \(", declaration)
        if name and os.path.dirname(path) == root:
            found.setdefault(os.path.basename(path), set()).add(name.group(1))
    return found


def compiled_definitions(gcc: str, include: Path, tmp: Path) -> dict[str, set[str]]:
    """header -> the functions GCC compiles out of it, by GCC's account."""
    source = tmp / "aux.c"
    source.write_text(HEAD)
    found: dict[str, set[str]] = {}
    for level in ("-O0", "-O2"):
        listing = tmp / f"aux{level}.txt"
        result = subprocess.run(
            [gcc, *GCC_FLAGS, level, "-fsyntax-only", "-aux-info", str(listing), str(source)],
            capture_output=True, text=True, cwd=tmp)
        if result.returncode != 0 or not listing.exists():
            raise RuntimeError(f"{gcc} {level} -aux-info: {result.stderr.strip()[:300]}")
        for header, names in aux_definitions_of(listing.read_text(), include).items():
            found.setdefault(header, set()).update(names)
    return found


def caller(index: int, name: str, returns: str, parameters: list[str], twin: bool) -> str:
    """One line of C that calls `name` with arguments of its own types."""
    declared, passed = [], []
    for position, parameter in enumerate(parameters):
        kind, spelled = parameter_of(parameter)
        if twin and kind.replace("const", "").strip() in IMMEDIATE_TYPES:
            rounding = spelled in ("__R", "__B_R", "__ROUND") or (
                spelled.endswith("R") and "round" in name)
            passed.append("8" if rounding else "1")
        else:
            declared.append(f"{kind} a{position}")
            passed.append(f"a{position}")
    call = f"{name}({', '.join(passed)})"
    body = f"{call};" if returns == "void" else f"return {call};"
    return f"{returns} t{index}({', '.join(declared) or 'void'}) {{ {body} }}"


def refused(command: list[str], lines: list[str], tmp: Path,
            mode: tuple[str, ...] = ("-fsyntax-only",)) -> set[int]:
    """The indexes of `lines` the compiler reports an error in, compiling
    as far as `mode` says (`OBJECT` for a whole compilation)."""
    alive, bad = list(range(len(lines))), set()
    source = tmp / "c.c"
    first = HEAD.count("\n") + 1
    while alive:
        source.write_text(HEAD + "\n".join(lines[i] for i in alive) + "\n")
        result = subprocess.run([*command, *mode, str(source)],
                                capture_output=True, text=True, cwd=tmp)
        if result.returncode == 0:
            return bad
        named = {alive[int(m.group(1)) - first]
                 for m in re.finditer(r"c\.c:(\d+):(?:\d+:)? (?:fatal )?error", result.stderr)
                 if 0 <= int(m.group(1)) - first < len(alive)}
        if not named:
            # An error without a line of ours: find it by halves.
            if len(alive) == 1:
                return bad | set(alive)
            half = len(alive) // 2
            left = refused(command, [lines[i] for i in alive[:half]], tmp, mode)
            right = refused(command, [lines[i] for i in alive[half:]], tmp, mode)
            return (bad | {alive[:half][i] for i in left}
                    | {alive[half:][i] for i in right})
        bad |= named
        alive = [i for i in alive if i not in named]
    return bad


def digest_of(names: list[str]) -> str:
    return hashlib.sha256("\n".join(sorted(names)).encode()).hexdigest()[:8]


def headers_of(include: Path) -> list[Path]:
    """GCC's x86 intrinsic headers, in the order of the record."""
    others = [include / name for name in OTHER_HEADERS if (include / name).exists()]
    return sorted([*include.glob("*intrin.h"), *others], key=lambda header: header.name)


def census(lccc: str, gcc: str, flags: list[str], only: set[str]):
    """({header: (callable in GCC, missing names, skipped names)},
    {header: the definitions GCC compiled that the reader did not find})."""
    include = Path(subprocess.run([gcc, "-print-file-name=include"], capture_output=True,
                                  text=True).stdout.strip())
    report, unread = {}, {}
    with tempfile.TemporaryDirectory() as directory:
        tmp = Path(directory)
        compiled = compiled_definitions(gcc, include, tmp)
        for header in headers_of(include):
            if only and header.name not in only:
                continue
            functions = functions_of(header.read_text(errors="replace"))
            lost = sorted(compiled.get(header.name, set()) - set(functions))
            if lost:
                unread[header.name] = lost
            names = sorted(functions)
            if not names:
                continue
            lines = [caller(i, name, *functions[name]) for i, name in enumerate(names)]
            skipped = refused([gcc, *GCC_FLAGS, "-O1"], lines, tmp)
            kept = [i for i in range(len(lines)) if i not in skipped]
            bad = refused([lccc, *flags, "-O1"], [lines[i] for i in kept], tmp, OBJECT)
            report[header.name] = (len(kept), [names[kept[i]] for i in sorted(bad)],
                                   [names[i] for i in sorted(skipped)])
    return report, unread


def expectation_text(report, gcc_version: str) -> str:
    total = sum(count for count, _, _ in report.values())
    missing = sum(len(names) for _, names, _ in report.values())
    lines = [
        "# scripts/x86_intrinsic_census.py --update",
        "# The intrinsics of GCC's x86 headers a program can call under GCC, and how",
        "# many of them it cannot call under lccc: header, callable, missing, digest",
        "# of the missing names (--list HEADER prints them).",
        f"gcc {gcc_version}",
        f"callable {total}",
        f"missing {missing}",
    ]
    lines += [f"header {header} {count} {len(names)} {digest_of(names)}"
              for header, (count, names, _) in sorted(report.items())]
    return "\n".join(lines) + "\n"


def failures_against(text: str, expected: str) -> list[str]:
    """What differs between a census and the record, line by line."""
    have = [line for line in text.splitlines() if line and not line.startswith("#")]
    want = [line for line in expected.splitlines() if line and not line.startswith("#")]
    return ([f"expected, not found:  {line}" for line in want if line not in have]
            + [f"found, not expected:  {line}" for line in have if line not in want])


def gcc_version_of(gcc: str) -> str:
    try:
        return subprocess.run([gcc, "-dumpfullversion"], capture_output=True,
                              text=True).stdout.strip()
    except OSError:
        return ""


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--lccc", default=str(ROOT / "target" / "release" / "lccc"))
    parser.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"))
    parser.add_argument("--flags", default=" ".join(LCCC_FLAGS),
                        help="the target lccc compiles the callers for")
    parser.add_argument("--check", action="store_true", help="compare with the record")
    parser.add_argument("--update", action="store_true", help="rewrite the record")
    parser.add_argument("--list", metavar="HEADER", action="append", default=[],
                        help="print the names lccc lacks of this header (repeatable)")
    args = parser.parse_args(argv)

    if not Path(args.lccc).exists():
        print(f"x86_intrinsic_census: no lccc at {args.lccc} (--lccc)", file=sys.stderr)
        return 2
    # The callers are compiled in a directory of their own.
    args.lccc = str(Path(args.lccc).resolve())
    version = gcc_version_of(args.gcc)
    if version != GCC_VERSION:
        print(f"x86_intrinsic_census: the headers are GCC {GCC_VERSION}'s; {args.gcc} is "
              f"{version or 'not runnable'} (--gcc, or GCC=)", file=sys.stderr)
        return 2
    if (args.check or args.update) and args.list:
        parser.error("--check and --update take the whole census")

    try:
        report, unread = census(args.lccc, args.gcc, args.flags.split(), set(args.list))
    except RuntimeError as error:
        print(f"x86_intrinsic_census: {error}", file=sys.stderr)
        return 2
    if unread:
        for header, names in sorted(unread.items()):
            print(f"x86_intrinsic_census: {header}: {len(names)} definitions GCC compiled were "
                  f"not read: {' '.join(names[:8])}{' ...' if len(names) > 8 else ''}",
                  file=sys.stderr)
        print("x86_intrinsic_census: the census would be short. The header reader "
              "(DEFINED_NAME, INLINE) has to learn this layout.", file=sys.stderr)
        return 2
    if args.list:
        for header in args.list:
            if header not in report:
                print(f"x86_intrinsic_census: GCC has no intrinsics in {header}",
                      file=sys.stderr)
                return 2
            print("\n".join(report[header][1]))
        return 0

    total = sum(count for count, _, _ in report.values())
    missing = sum(len(names) for _, names, _ in report.values())
    skipped = sum(len(names) for _, _, names in report.values())
    for header, (count, names, _) in sorted(report.items()):
        if names:
            print(f"{header:32} {count:5} callable in GCC, {len(names):5} not in lccc")
    print(f"{total} intrinsics of {len(report)} headers are callable in GCC {version} "
          f"({skipped} callers GCC refuses were dropped); lccc lacks {missing} "
          f"({100 * missing / max(total, 1):.1f}%)")
    text = expectation_text(report, version)
    if args.update:
        EXPECTED.parent.mkdir(parents=True, exist_ok=True)
        EXPECTED.write_text(text)
        print(f"wrote {EXPECTED.relative_to(ROOT)}")
        return 0
    if args.check:
        if not EXPECTED.exists():
            print(f"x86_intrinsic_census: no record at {EXPECTED.relative_to(ROOT)} (--update)",
                  file=sys.stderr)
            return 1
        problems = failures_against(text, EXPECTED.read_text())
        for problem in problems:
            print("  " + problem)
        if problems:
            print("x86_intrinsic_census: the census is not the record's. An intrinsic that "
                  "went missing is a regression; one that was added is recorded with "
                  "--update.")
            return 1
        print("x86_intrinsic_census: as recorded")
    return 0


if __name__ == "__main__":
    sys.exit(main())
