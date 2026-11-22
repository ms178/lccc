#!/usr/bin/env python3
"""Differential gate for the prefetch instructions on x86.

A prefetch computes nothing, so no test that compares values can see one:
`_mm_prefetch` was a macro that evaluated its arguments and emitted nothing
for as long as this compiler had the header, and every execution suite
passed.  What a prefetch is, is the instruction, so that is what this gate
compares, with GCC 16.2, statement by statement:

  * `__builtin_prefetch(p, rw, locality)` for all twelve pairs and the two
    shorter forms;
  * `_mm_prefetch(p, I)` for every selector of `enum _mm_hint` by name and
    for the raw values 0..15;
  * `_m_prefetchw(p)` (with `<immintrin.h>`, which costs GCC a third of a
    second to read: asked of the option rows, not of every CPU);

under every target that changes the answer: every `-march=` name of the
CPU table, as a 64-bit and as a 32-bit target, each plain, without SSE,
with PRFCHW (`prefetchw` for a write) and with MOVRS (`prefetchrst2` for a
shared read of locality 1), and the option pairs that take a grant back.
A 32-bit target has the instructions only where the CPU or an option has
them, so most of the rows that differ from one another are there.

Each function of the fixture holds one statement, and the gate compares the
prefetch mnemonics each compiler writes into it: the same instruction, the
same number of times (once, or not at all where the target has none).

One difference is by design, and it is stated to GCC instead of excused
afterwards: lccc has no 3DNow! (`-m3dnow` is refused), so GCC is asked
with `-mno-3dnow`.  On a CPU with 3DNow! (the K6-2, the Athlons, the
Geode, the WinChip 2, the C3, the K8s without SSE2 granted) GCC otherwise
writes the 3DNow! `prefetch`/`prefetchw`; lccc writes the SSE forms where
the CPU has them and nothing where it does not, which is what GCC writes
without 3DNow!.  The rows this changes are counted in the last line.

The selectors GCC refuses are compared too: a prefetch of code with a
locality it has no instruction for (`_mm_prefetch(p, 16)`) is an error in
both, and a hint that is no constant is one in both.

Usage: check_x86_prefetch.py LCCC [--gcc GCC] [--lccc-i686 LCCC_I686] [-v]
Exit status is the number of failing rows (0 = clean).
"""
from __future__ import annotations

import argparse
import concurrent.futures
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ENV = {**os.environ, "LC_ALL": "C", "LANG": "C"}
GCC_VERSION = "16.2.0"

# Selectors of `enum _mm_hint` in GCC 16.2, and what the header makes of
# them: (rw, locality, instruction) = ((I & 0xC) >> 2, I & 3, (I & 0x10) >> 4).
NAMED_HINTS = ("_MM_HINT_T0", "_MM_HINT_T1", "_MM_HINT_T2", "_MM_HINT_NTA", "_MM_HINT_ET0",
               "_MM_HINT_RST2", "_MM_HINT_IT0", "_MM_HINT_IT1")

ROOT = Path(__file__).resolve().parents[1]
TABLE = ROOT / "src" / "backend" / "x86" / "arch_table.rs"

# What every CPU is asked with: nothing, and each option that moves a
# prefetch instruction in or out.
PER_CPU = ([], ["-mno-sse"], ["-msse"], ["-mprfchw"], ["-mmovrs"], ["-mno-sse", "-mprfchw"])
# Asked once per mode, without a CPU name and with the two CPUs that have
# MOVRS and PRFCHW of their own: the pairs that grant and take back.
PAIRS = (
    ["-mprfchw", "-mmovrs"],
    ["-mmovrs", "-mno-movrs"],
    ["-mprfchw", "-mno-prfchw"],
    ["-mno-sse", "-mmovrs"],
    ["-mno-sse", "-mprfchw", "-mmovrs"],
    ["-mgeneral-regs-only"],
    ["-march=native"],
    ["-march=diamondrapids", "-mno-movrs"],
    ["-march=diamondrapids", "-mno-prfchw"],
    ["-march=novalake", "-mno-movrs", "-mno-prfchw"],
    ["-march=diamondrapids", "-march=haswell"],
    ["-march=haswell", "-march=diamondrapids"],
    ["-march=i686", "-mprfchw", "-mmovrs"],
    ["-march=i686", "-msse", "-mno-sse"],
    ["-march=i686", "-mno-sse", "-msse"],
    ["-march=i686", "-mgeneral-regs-only"],
    ["-march=pentium3", "-mno-sse", "-mno-prfchw"],
    ["-msse", "-mno-sse"],
    ["-march=raptorlake", "-mno-sse", "-mno-prfchw"],
    ["-march=i586", "-march=haswell", "-mno-sse"],
    ["-march=haswell", "-march=i586"],
)


def cpus() -> list[tuple[str, bool]]:
    """(name, is a 64-bit CPU) for every row of the CPU table."""
    found = [(match.group(1), match.group(2) == "Y")
             for match in re.finditer(r'^\s*row\("([^"]+)",\s+([YN]),', TABLE.read_text(), re.M)]
    if len(found) < 100:
        raise SystemExit(f"{TABLE}: only {len(found)} rows read")
    return found


def option_rows() -> list[tuple[str, list[str], bool]]:
    """(label, flags, 32-bit) for the targets without a CPU of their own."""
    asked = []
    for narrow in (False, True):
        mode = "-m32" if narrow else "-m64"
        for flags in ([], *PER_CPU[1:], *PAIRS):
            if not narrow and any(flag in ("-march=i686", "-march=i586", "-march=pentium3")
                                  for flag in flags):
                continue
            asked.append((" ".join([mode, *flags]), list(flags), narrow))
    return asked


def rows() -> list[tuple[str, list[str], bool]]:
    """(label, flags, 32-bit) for every target asked."""
    asked = option_rows()
    for narrow in (False, True):
        mode = "-m32" if narrow else "-m64"
        for name, wide in cpus():
            if narrow or wide:
                for flags in PER_CPU:
                    asked.append((" ".join([mode, f"-march={name}", *flags]),
                                  [f"-march={name}", *flags], narrow))
    return asked


def fixture() -> tuple[str, list[str]]:
    """The source, and the name of every function in it."""
    lines = ["#include <xmmintrin.h>"]
    names = []

    def add(name: str, statement: str) -> None:
        names.append(name)
        lines.append(f"void {name}(const char *p) {{ {statement} }}")

    add("b_default", "__builtin_prefetch(p);")
    add("b_read", "__builtin_prefetch(p, 0);")
    add("b_write", "__builtin_prefetch(p, 1);")
    add("b_shared", "__builtin_prefetch(p, 2);")
    for rw in range(3):
        for locality in range(4):
            add(f"b_{rw}_{locality}", f"__builtin_prefetch(p, {rw}, {locality});")
    for hint in NAMED_HINTS:
        add("m" + hint.lower(), f"_mm_prefetch(p, {hint});")
    for value in range(16):
        add(f"m_raw_{value}", f"_mm_prefetch(p, {value});")
    add("m_expression", "_mm_prefetch(p + 64, _MM_HINT_T0 | 4);")
    add("m_enum_object", "enum _mm_hint hint = _MM_HINT_T2; (void)hint; "
                         "_mm_prefetch(p, _MM_HINT_T2);")
    add("m_twice", "_mm_prefetch(p, _MM_HINT_T0); _mm_prefetch(p + 64, _MM_HINT_NTA);")
    return "\n".join(lines) + "\n", names


# The one prefetch intrinsic outside <xmmintrin.h>.
WRITE_FIXTURE = ("#include <immintrin.h>\n"
                 "void w_prefetchw(const char *p) { _m_prefetchw(p); }\n"
                 "void w_volatile(volatile const int *p) { _m_prefetchw(p + 16); }\n")
WRITE_NAMES = ["w_prefetchw", "w_volatile"]


# What both compilers have to refuse, and the text GCC refuses it with.
REFUSED = (
    ("code, locality 0", "void f(const char *p) { _mm_prefetch(p, 16); }",
     "invalid third argument"),
    ("code, locality 1", "void f(const char *p) { _mm_prefetch(p, 17); }",
     "invalid third argument"),
    ("selector of the running program",
     "void f(const char *p, int i) { _mm_prefetch(p, i); }",
     "second, third and fourth argument must be a const"),
    ("builtin: locality of the running program",
     "void f(const char *p, int i) { __builtin_ia32_prefetch(p, 0, i, 0); }",
     "second, third and fourth argument must be a const"),
    ("generic builtin: rw of the running program",
     "void f(const char *p, int i) { __builtin_prefetch(p, i, 3); }",
     "second argument to '__builtin_prefetch' must be a constant"),
)

# What both compilers take and warn about, and the text of the warning.
WARNED = (
    ("rw 3", "void f(const char *p) { _mm_prefetch(p, 12); }",
     "invalid second argument to '__builtin_ia32_prefetch'; using zero"),
    ("code", "void f(const char *p) { _mm_prefetch(p, _MM_HINT_IT0); }",
     "instruction prefetch applies when in 64-bit mode with RIP-relative addressing and "
     "option '-mprefetchi'; they stay NOPs otherwise"),
    ("generic builtin: locality 4", "void f(const char *p) { __builtin_prefetch(p, 0, 4); }",
     "invalid third argument to '__builtin_prefetch'; using zero"),
)

# GCC's selectors are enumerators; Clang's, ICC's and MSVC's are macros, and
# programs written against those ask `#ifdef _MM_HINT_ET0`. lccc's are both.
VISIBLE = ("#include <xmmintrin.h>\n"
           + "".join(f"#ifndef {hint}\n#error {hint} is not visible to the preprocessor\n#endif\n"
                     for hint in NAMED_HINTS)
           + "enum _mm_hint as_a_type = _MM_HINT_T0;\n"
           + "_Static_assert(_MM_HINT_T0 == 3 && _MM_HINT_T1 == 2 && _MM_HINT_T2 == 1 "
             "&& _MM_HINT_NTA == 0 && _MM_HINT_ET0 == 7 && _MM_HINT_RST2 == 9 "
             "&& _MM_HINT_IT0 == 19 && _MM_HINT_IT1 == 18, \"GCC 16.2's values\");\n")

PREFETCH = re.compile(r"^\s+(prefetch[a-z0-9]*)\b", re.M)
LABEL = re.compile(r"^([A-Za-z_][A-Za-z0-9_]*):", re.M)


def prefetches_of(assembly: str, names: list[str]) -> dict[str, list[str]]:
    """function -> the prefetch mnemonics written between its label and the next."""
    wanted = set(names)
    found: dict[str, list[str]] = {name: [] for name in names}
    current = None
    for line in assembly.splitlines():
        label = LABEL.match(line)
        if label:
            # A local label (`.L3:`) does not match; another global ends the function.
            current = label.group(1) if label.group(1) in wanted else None
            continue
        instruction = PREFETCH.match(line)
        if instruction and current is not None:
            found[current].append(instruction.group(1))
    return found


def assembly_of(compiler: list[str], flags: list[str], source: Path) -> tuple[str | None, str]:
    result = subprocess.run([*compiler, "-O2", "-w", *flags, "-S", str(source), "-o", "-"],
                            capture_output=True, text=True, env=ENV)
    return (result.stdout if result.returncode == 0 else None), result.stderr


def has_3dnow(compiler: list[str], flags: list[str]) -> bool:
    """Does GCC enable 3DNow! for this target?"""
    result = subprocess.run([*compiler, *flags, "-dM", "-E", "-x", "c", os.devnull],
                            capture_output=True, text=True, env=ENV)
    return "__3dNOW__" in result.stdout


def first_error(said: str) -> str:
    lines = [line for line in said.splitlines() if "error" in line]
    return (lines or said.strip().splitlines() or ["no message"])[0][:200]


def differences(reference: dict[str, list[str]], ours: dict[str, list[str]]) -> list[str]:
    return [f"{name}: GCC {' '.join(reference[name]) or 'nothing'}, "
            f"lccc {' '.join(ours[name]) or 'nothing'}"
            for name in reference if reference[name] != ours[name]]


def diagnostics_of(compiler: list[str], source: Path) -> tuple[int, str]:
    result = subprocess.run([*compiler, "-O2", "-c", str(source), "-o", os.devnull],
                            capture_output=True, text=True, env=ENV)
    return result.returncode, result.stderr


def gcc_version_of(gcc: str) -> str:
    try:
        return subprocess.run([gcc, "-dumpfullversion"], capture_output=True,
                              text=True).stdout.strip()
    except OSError:
        return ""


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("lccc")
    parser.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"))
    parser.add_argument("--lccc-i686", default=None,
                        help="the 32-bit driver (default: lccc-i686 beside LCCC)")
    parser.add_argument("-v", "--verbose", action="store_true")
    args = parser.parse_args(argv)

    lccc = str(Path(args.lccc).resolve())
    lccc_i686 = args.lccc_i686 or str(Path(lccc).with_name("lccc-i686"))
    for binary in (lccc, lccc_i686):
        if not Path(binary).exists():
            print(f"check_x86_prefetch: no compiler at {binary}", file=sys.stderr)
            return 2
    version = gcc_version_of(args.gcc)
    if version != GCC_VERSION:
        print(f"check_x86_prefetch: the reference is GCC {GCC_VERSION}; {args.gcc} is "
              f"{version or 'not runnable'} (--gcc, or GCC=)", file=sys.stderr)
        return 2

    failures = 0
    text, names = fixture()
    asked = rows()
    with tempfile.TemporaryDirectory() as directory:
        source = Path(directory) / "prefetch.c"
        source.write_text(text)
        write_source = Path(directory) / "prefetchw.c"
        write_source.write_text(WRITE_FIXTURE)

        def examine(row, fixture_source: Path, functions: list[str]):
            """(error or None, what GCC selects, whether 3DNow! would change it,
            the statements that differ)."""
            _, flags, narrow = row
            ours_compiler = [lccc_i686] if narrow else [lccc]
            reference_compiler = [args.gcc, "-m32"] if narrow else [args.gcc]
            reference, said = assembly_of(reference_compiler, [*flags, "-mno-3dnow"],
                                          fixture_source)
            if reference is None:
                return f"GCC does not compile the fixture: {first_error(said)}", {}, False, []
            ours, said = assembly_of(ours_compiler, flags, fixture_source)
            if ours is None:
                return f"lccc does not compile the fixture: {first_error(said)}", {}, False, []
            expected = prefetches_of(reference, functions)
            # The fork, counted: what GCC writes with the CPU's own 3DNow!.
            changed = False
            if has_3dnow(reference_compiler, flags):
                native, _ = assembly_of(reference_compiler, flags, fixture_source)
                changed = native is not None and prefetches_of(native, functions) != expected
            return None, expected, changed, differences(expected, prefetches_of(ours, functions))

        selected: set[str] = set()
        silent = with_3dnow = 0
        jobs = ([(row, source, names) for row in asked]
                + [(row, write_source, WRITE_NAMES) for row in option_rows()])
        with concurrent.futures.ThreadPoolExecutor(max_workers=os.cpu_count() or 1) as pool:
            results = list(pool.map(lambda job: examine(*job), jobs))
        for (row, _, functions), (error, expected, changed, problems) in zip(jobs, results):
            label = row[0] + (" (_m_prefetchw)" if functions is WRITE_NAMES else "")
            if error:
                print(f"FAIL {label}: {error}")
                failures += 1
                continue
            selected |= {mnemonic for found in expected.values() for mnemonic in found}
            silent += not any(expected.values())
            with_3dnow += changed
            if problems:
                failures += 1
                print(f"FAIL {label}: {len(problems)} of {len(functions)} statements")
                for problem in problems[:None if args.verbose else 4]:
                    print(f"    {problem}")
                if len(problems) > 4 and not args.verbose:
                    print(f"    ... {len(problems) - 4} more (-v)")
            elif args.verbose:
                emitted = sum(1 for found in expected.values() if found)
                print(f"ok   {label}: {emitted} of {len(functions)} statements have an "
                      f"instruction")
        # The matrix is a test of the 32-bit rule only if some target has none.
        if not silent:
            print("FAIL the matrix: no target is without a prefetch instruction")
            failures += 1

        # The matrix is a test only if the reference selects every instruction in it.
        for mnemonic in ("prefetcht0", "prefetcht1", "prefetcht2", "prefetchnta", "prefetchw",
                         "prefetchrst2"):
            if mnemonic not in selected:
                print(f"FAIL the matrix: GCC selects {mnemonic} in no row")
                failures += 1

        for label, body, message in REFUSED:
            source.write_text("#include <xmmintrin.h>\n" + body + "\n")
            for who, compiler in (("GCC", [args.gcc]), ("lccc", [lccc])):
                status, said = diagnostics_of(compiler, source)
                if status == 0 or "error" not in said or message not in said:
                    failures += 1
                    print(f"FAIL refused, {label}: {who} "
                          f"{'compiles it' if status == 0 else 'does not say: ' + message}")
        source.write_text(VISIBLE)
        status, said = diagnostics_of([lccc], source)
        if status != 0:
            failures += 1
            print(f"FAIL the selectors as macros and as an enumeration: {first_error(said)}")
        for label, body, message in WARNED:
            source.write_text("#include <xmmintrin.h>\n" + body + "\n")
            for who, compiler in (("GCC", [args.gcc]), ("lccc", [lccc])):
                status, said = diagnostics_of(compiler, source)
                if status != 0 or "warning" not in said or message not in said:
                    failures += 1
                    print(f"FAIL warned, {label}: {who} "
                          f"{'refuses it' if status != 0 else 'does not say: ' + message}")

    statements = len(names) * len(asked) + len(WRITE_NAMES) * len(option_rows())
    if failures:
        print(f"check_x86_prefetch: {failures} failing rows of {len(asked)} targets")
    else:
        print(f"check_x86_prefetch: {statements} statements in {len(asked)} targets select "
              f"GCC {version}'s instruction; {len(REFUSED)} refusals and {len(WARNED)} "
              f"warnings are GCC's")
    print(f"check_x86_prefetch: GCC was asked with -mno-3dnow (lccc has no 3DNow!); with the "
          f"CPU's 3DNow! it writes other instructions in {with_3dnow} of the targets")
    return min(failures, 255)


if __name__ == "__main__":
    sys.exit(main())
