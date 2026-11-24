#!/usr/bin/env python3
"""What objtool reports about lccc's objects and not about GCC's.

objtool (the Linux kernel's object validator, scripts/ensure_objtool.sh)
decodes every instruction of an x86-64 object and follows every path with
the stack state it implies. It was written for kernel code, so on ordinary
programs it also reports constructs it merely does not model (stack
realignment, `alloca`, calls that do not return). The useful question is
therefore differential: every x86-64 fixture of the regression corpus is
compiled by lccc and by GCC with the fixture's own flags plus the kernel's
`-fno-pie`, and a warning counts when lccc's object draws it in a function
for which GCC's object draws none.

    scripts/objtool_differential.py --lccc target/fastbuild/lccc --check

`--check` fails on a warning that is not in `TRIAGED` below, and on a
`TRIAGED` entry that no longer occurs. The first run of this tool (1,407
fixtures) reported one real defect family -- the CPUID probe helper had no
ELF size and its words no type, fixed and pinned by
tests/regression/check_symbol_annotations.py -- and no stack-state warning
at all: no `ret` with a moved stack pointer, no two paths that meet at
different depths, in any function of the corpus.

Not a CI gate: the runners have no objtool. It is the tool to run after a
change to frame layout, prologue/epilogue emission, tail calls, or any
peephole pass that reasons about `%rsp`.
"""

from __future__ import annotations

import argparse
import collections
import concurrent.futures
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CORPUS = ROOT / "tests" / "regression"

# (fixture, function, warning) -> why it is no defect. Each is a jump that
# objtool cannot follow because its target is an address taken in C.
COMPUTED = "the instruction is the target of a computed goto / label address"
SETJMP = "the instruction is a `__builtin_setjmp` receiver, reached by `__builtin_longjmp`"
ASM_GOTO = "the instruction is reached through the fixture's own assembly jump table"
NONLOCAL = ("a nested function leaves through a non-local goto: a jump into its "
            "parent's body (GCC's code draws a different warning for the same jump)")
TRIAGED = {
    ("asm_goto_jump_table", "branch_taken", "unreachable instruction"): ASM_GOTO,
    ("asm_goto_jump_table", "branch_two", "unreachable instruction"): ASM_GOTO,
    ("builtin_setjmp_longjmp_dce", "foo", "unreachable instruction"): SETJMP,
    ("builtin_setjmp_longjmp_x86", "exercise", "unreachable instruction"): SETJMP,
    ("cet_computed_goto_targets", "local_dispatch", "unreachable instruction"): COMPUTED,
    ("static_label_difference_alias", "dispatch", "unreachable instruction"): COMPUTED,
    ("nested_fn_chain_goto_cross", "leave_through_goto.y", "falls through to next function"): NONLOCAL,
    ("nested_fn_static_chain_regalloc", "nlg_depth.rec", "falls through to next function"): NONLOCAL,
    ("nested_goto_shared_label", "parent.leave", "falls through to next function"): NONLOCAL,
    ("nested_goto_shared_label", "parent.sometimes", "falls through to next function"): NONLOCAL,
    ("nested_nonlocal_goto_callee_saved", "exercise.transfer", "falls through to next function"): NONLOCAL,
    ("nested_nonlocal_goto_global_register", "main.transfer", "falls through to next function"): NONLOCAL,
}

WARNING = re.compile(r"^[^:]+: (?:warning|error): objtool: (.*)$")
FUNCTION = re.compile(r"^([A-Za-z_.$][A-Za-z0-9_.$]*)(?:\+0x[0-9a-f]+|\(\))?:? ?")


def fixture_flags(source: Path) -> list[str]:
    flags = source.with_suffix(".flags")
    return flags.read_text().split() if flags.exists() else ["-O2"]


def warnings(objtool: str, compiler: str, source: Path, flags: list[str], obj: Path):
    """The (function, class) pairs objtool reports, or None when the source
    does not compile."""
    built = subprocess.run([compiler, *flags, "-fno-pie", "-w", "-c", str(source), "-o", str(obj)],
                           capture_output=True, text=True, check=False)
    if built.returncode != 0:
        return None
    report = subprocess.run([objtool, "--orc", "--dry-run", str(obj)],
                            capture_output=True, text=True, check=False).stderr
    found = set()
    for line in report.splitlines():
        match = WARNING.match(line.strip())
        if not match:
            continue
        message = match.group(1)
        function = FUNCTION.match(message)
        if not function:
            found.add(("", message))
            continue
        kind = re.sub(r"0x[0-9a-f]+|\d+", "N", message[function.end():])
        kind = re.sub(r"next function .*", "next function", kind)
        found.add((function.group(1), kind))
    return found


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--lccc", default=str(ROOT / "target/fastbuild/lccc"))
    parser.add_argument("--gcc", default=shutil.which("gcc-16") or "gcc")
    parser.add_argument("--objtool", default=None,
                        help="default: the binary scripts/ensure_objtool.sh provides")
    parser.add_argument("--filter", default="", help="only fixtures whose name contains this")
    parser.add_argument("-j", type=int, default=2)
    parser.add_argument("--check", action="store_true",
                        help="fail on a warning outside TRIAGED, or a stale TRIAGED entry")
    args = parser.parse_args()
    objtool = args.objtool or subprocess.run(
        ["bash", str(ROOT / "scripts/ensure_objtool.sh")], capture_output=True, text=True,
        check=True).stdout.strip()

    sources = [s for s in sorted(CORPUS.glob("*.c")) if args.filter in s.name]
    seen = collections.defaultdict(list)   # (class, verdict) -> ["fixture: function"]
    occurred = set()
    compiled = 0
    with tempfile.TemporaryDirectory(prefix="lccc-objtool-") as tmp:
        def one(source: Path):
            flags = fixture_flags(source)
            if any(flag in ("-m32", "-m16", "-mx32") for flag in flags):
                return source.stem, None, None   # objtool reads x86-64 objects only
            ours = warnings(objtool, args.lccc, source, flags, Path(tmp) / f"{source.stem}.l.o")
            theirs = warnings(objtool, args.gcc, source, flags, Path(tmp) / f"{source.stem}.g.o")
            return source.stem, ours, theirs

        with concurrent.futures.ThreadPoolExecutor(args.j) as pool:
            for stem, ours, theirs in pool.map(one, sources):
                if ours is None:
                    continue
                compiled += 1
                gcc_functions = {function for function, _ in theirs or ()}
                for function, kind in sorted(ours):
                    if theirs and (function, kind) in theirs:
                        continue
                    if function in gcc_functions:
                        verdict = "objtool also objects to GCC's code for this function"
                    elif (stem, function, kind) in TRIAGED:
                        verdict = "triaged"
                        occurred.add((stem, function, kind))
                    else:
                        verdict = "LCCC ONLY"
                    seen[(kind, verdict)].append(f"{stem}: {function}")

    print(f"{compiled} x86-64 fixtures compiled by lccc ({Path(args.lccc).name}) and judged by objtool")
    for (kind, verdict), where in sorted(seen.items(), key=lambda item: (-len(item[1]), item[0])):
        print(f"{len(where):4}  {kind}  [{verdict}]")
        if verdict == "LCCC ONLY":
            for place in where:
                print(f"        {place}")
    fresh = sum(len(where) for (_, verdict), where in seen.items() if verdict == "LCCC ONLY")
    stale = sorted(set(TRIAGED) - occurred) if not args.filter else []
    for entry in stale:
        print(f"stale TRIAGED entry (no longer reported): {entry}")
    verdict = "FAIL" if fresh or stale else "PASS"
    print(f"objtool-differential: {fresh} warnings only lccc draws, {len(occurred)} triaged, "
          f"{len(stale)} stale, {verdict}")
    return 1 if args.check and (fresh or stale) else 0


if __name__ == "__main__":
    sys.exit(main())
