#!/usr/bin/env python3
"""GCC 16 differential for C digraph tokens, directives and macro operators.

The test preserves the source spelling in stringification, checks maximal
munch and token separation, and executes native programs. It deliberately does
not claim UCN, trigraph, pedantic-mode or general preprocessor parity.
"""
from __future__ import annotations

import argparse
import re
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

HEADER = r'''%:ifndef DIGRAPH_HEADER
%:define DIGRAPH_HEADER
%:define FROM_HEADER \
    11
%:endif
'''

PROGRAM = r'''%:include "digraph.h"
%:define PASTE(a,b) a %:%: b
%:define STR(x) %:x
%:define EXPAND_STR(x) STR(x)
%:define STRING_AND_TEXT(x) "prefix %: # %:%: ## " %:x
%:define MAKE_OPEN <%
%:define MAKE_CLOSE %>
%:define MULTI /* block comment crosses
                  a physical line */ 7
%:if FROM_HEADER != 11 || MULTI != 7
%:error digraph conditional failed
%:endif
%:line 301 "virtual-digraph.c"
enum <% DIGRAPH_LINE = __LINE__ %>;
int values<:3:> = <%2, 3, 5%>;
static int same(const char *a, const char *b) <%
    while (*a && *a == *b) a++, b++;
    return *a == *b;
%>
int main(void) MAKE_OPEN
    if (values PASTE(<,:) 0 :> != 2 || values<:2:> != 5) return 1;
    if (!same(STR(<: :> <% %> %: %:%:), "<: :> <% %> %: %:%:")) return 2;
    if (!same(EXPAND_STR(PASTE(<,:)), "<:")) return 3;
    if (!same(STRING_AND_TEXT(test), "prefix %: # %:%: ## test")) return 4;
    if (DIGRAPH_LINE != 301) return 5;
    return FROM_HEADER + MULTI == 18 ? 0 : 6;
MAKE_CLOSE
'''

PREPROCESS = r'''%:define S(x) %:x
%:define C(a,b) a %:%: b
%:define LT <
%:define COLON :
%:define PC %
%:define GT >
%:define HASH %:
%:define OVERLAP(x) <%: x
%:define SHIFT(x) <<%:x
DIGRAPH_STRING S(<: :> <% %> %: %:%:)
DIGRAPH_EXPANDED S(C(<,:))
DIGRAPH_SEP_A LT COLON
DIGRAPH_SEP_B PC GT
DIGRAPH_SEP_C HASH HASH
DIGRAPH_OVERLAP OVERLAP(foo)
DIGRAPH_SHIFT SHIFT(foo)
'''

C23_NUMBERS = r'''%:define NUMBER_AND_STRING(x) (1'234 + sizeof(%:x))
int main(void) <% return NUMBER_AND_STRING(q) == 1236 ? 0 : 1; %>
'''

NEGATIVE = {
    "invalid-stringize": "%:define BAD(x) %: not_a_parameter\n",
    "missing-stringize-operand": "%:define BAD(x) %:\n",
    "paste-is-not-directive": "%:%:define VALUE 1\nint x;\n",
}


def run(command: list[str]) -> subprocess.CompletedProcess[str]:
    try:
        return subprocess.run(command, text=True, capture_output=True, timeout=30, check=False)
    except (OSError, subprocess.TimeoutExpired) as error:
        raise RuntimeError(f"cannot run {command!r}: {error}") from error


def require_success(command: list[str]) -> subprocess.CompletedProcess[str]:
    result = run(command)
    if result.returncode != 0:
        raise RuntimeError(f"{command!r} failed ({result.returncode}):\n{result.stderr}")
    return result


def markers(output: str) -> dict[str, str]:
    found = {}
    for line in output.splitlines():
        match = re.match(r"\s*(DIGRAPH_\w+)\s+(.*)", line)
        if match:
            found[match.group(1)] = re.sub(r"\s+", " ", match.group(2).strip())
    return found


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lccc", default=str(ROOT / "target/fastbuild/lccc"))
    parser.add_argument("--gcc", default="gcc-16")
    parser.add_argument("--m32", action="store_true", help="also execute i686 binaries")
    args = parser.parse_args()
    gcc = args.gcc
    lccc = str(Path(args.lccc).resolve())
    if require_success([gcc, "-dumpfullversion"]).stdout.strip() != "16.2.0":
        raise RuntimeError("this regression requires GCC 16.2.0")
    compilers = [("gcc", gcc), ("lccc", lccc)]
    with tempfile.TemporaryDirectory(prefix="lccc-digraphs-") as directory:
        work = Path(directory)
        (work / "digraph.h").write_text(HEADER)
        source = work / "digraph.c"
        source.write_text(PROGRAM)
        execution_count = 0
        for compiler_name, compiler in compilers:
            for dialect in ("c99", "c11", "c17", "c23", "gnu89"):
                for arch in ([], ["-m32"]) if args.m32 else ([],):
                    binary = work / f"{compiler_name}-{dialect}-{'32' if arch else '64'}"
                    require_success([compiler, f"-std={dialect}", *arch, str(source), "-o", str(binary)])
                    require_success([str(binary)])
                    execution_count += 1
            c23 = work / "c23.c"
            c23.write_text(C23_NUMBERS)
            binary = work / f"{compiler_name}-c23-number"
            require_success([compiler, "-std=c23", str(c23), "-o", str(binary)])
            require_success([str(binary)])
            execution_count += 1

        # The same source must reach object emission on each integrated target;
        # this does not claim cross-target execution or an independent ABI oracle.
        bin_dir = Path(lccc).parent
        targets = (
            ("x86-64", Path(lccc), []),
            ("i686", bin_dir / "lccc-i686", []),
            ("aarch64", bin_dir / "lccc-arm", []),
            ("riscv64", bin_dir / "lccc-riscv", []),
            ("riscv32", bin_dir / "lccc-riscv", ["-march=rv32gc", "-mabi=ilp32d"]),
        )
        for name, compiler, flags in targets:
            require_success([str(compiler), "-std=c23", *flags, "-c", str(source),
                             "-o", str(work / f"{name}.o")])

        pp = work / "preprocess.c"
        pp.write_text(PREPROCESS)
        preprocessed = {}
        for name, compiler in compilers:
            preprocessed[name] = markers(require_success([compiler, "-std=c23", "-E", "-P", str(pp)]).stdout)
        expected = {
            "DIGRAPH_STRING": '"<: :> <% %> %: %:%:"',
            "DIGRAPH_EXPANDED": '"C(<,:)"',
            "DIGRAPH_SEP_A": "< :",
            "DIGRAPH_SEP_B": "% >",
            "DIGRAPH_SEP_C": "%: %:",
            "DIGRAPH_OVERLAP": "<%: foo",
            "DIGRAPH_SHIFT": '<<"foo"',
        }
        if preprocessed["gcc"] != expected or preprocessed["lccc"] != expected:
            raise RuntimeError(f"preprocessing mismatch: expected {expected!r}, got {preprocessed!r}")

        # Exercise overlapping and adjacent punctuators. In particular,
        # `<%:x` starts with `<%`, while `<<%:x` starts with `<<` then `%:`.
        prefixes = ("<", "<<", "<<=", "<=", "<:", "<%", ":>", "%>", ">>", "%",
                    "==", "+", "-", "*", "/", "&", "|", "^", "!", ":")
        matrix = work / "punctuator-matrix.c"
        rows = [(prefix, operator) for operator in ("#", "%:") for prefix in prefixes]
        matrix.write_text("\n".join(
            f"%:define M{i}(x) {prefix}{operator}x\nMATRIX{i} M{i}(foo)"
            for i, (prefix, operator) in enumerate(rows)) + "\n")
        outputs = {}
        for name, compiler in compilers:
            output = require_success([compiler, "-std=c23", "-E", "-P", str(matrix)]).stdout
            outputs[name] = [line for line in output.splitlines() if line.startswith("MATRIX")]
        if len(outputs["gcc"]) != len(rows) or outputs["gcc"] != outputs["lccc"]:
            raise RuntimeError(f"overlapping-punctuator matrix mismatch: {outputs!r}")

        for case, text in NEGATIVE.items():
            bad = work / f"{case}.c"
            bad.write_text(text)
            for name, compiler in compilers:
                result = run([compiler, "-std=c23", "-fsyntax-only", str(bad)])
                if result.returncode == 0:
                    raise RuntimeError(f"{name} accepted invalid {case}: {result.stdout!r}")

        error = work / "error.c"
        error.write_text("  %:error fail\\n")
        for name, compiler in compilers:
            result = run([compiler, "-std=c23", "-fsyntax-only", str(error)])
            if result.returncode == 0 or f"{error}:1:5: error:" not in result.stderr:
                raise RuntimeError(f"{name}: wrong digraph #error location: {result.stderr!r}")

    print(f"PASS: C digraphs: {execution_count} GCC/LCCC executions across five dialects, "
          "five integrated targets, seven token controls, 40 punctuator cases, "
          "three invalid inputs and a diagnostic position")


if __name__ == "__main__":
    try:
        main()
    except RuntimeError as error:
        raise SystemExit(f"FAIL: {error}")
