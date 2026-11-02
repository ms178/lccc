#!/usr/bin/env python3
"""Name the peephole sub-pass that corrupted the assembly.

`CCC_PEEPHOLE_TRACE=<dir>` makes the x86-64 peephole driver write the assembly
text after every sub-pass that reported a change, as
``<dir>/<seq>-p<iter>-<pass>.s``.  Reading that sequence in order and comparing
consecutive dumps is the ONLY reliable way to find a faulty rewrite, because
``CCC_PEEPHOLE_SKIP`` bisection reports the wrong pass whenever passes enable
one another: the pass that fires only because an earlier one rewrote its input
shows up as the "cause".  The driver's own comment says so; this is the tool it
refers to.

Two checks per dump, in order:

  assemble  Hand the dump to a real assembler.  A pass that emits an
            instruction with no encoding fails here, and the pass named in the
            filename of the FIRST dump that fails to assemble is the culprit.
            This is the dynamic check and it covers the whole class of
            "mis-encoded operand" bugs, not just the one below.

  operands  Report the first dump whose memory operands cannot be encoded:
            more than two register fields, more than three fields total, or an
            illegal scale.  Example: ``leaq 0(,%r11,4, %r10, 1), %r9``, which
            has two index registers.  The integrated assembler used to
            silently TRUNCATE that to ``leaq 0(,%r11,4), %r9``, so this check
            is worth having independently of the assembler.  ``--pattern``
            overrides it with a regex when hunting something else.

Why the truncation mattered: `fold_lea_into_load` spliced a producer `leaq`'s
address text into the base slot of an indexed consumer.  For
`leaq 0(,%r11,4), %r8` followed by `leaq (%r8, %r10, 1), %r9` it produced
`leaq 0(,%r11,4, %r10, 1), %r9`, which assembled as `leaq 0(,%r11,4), %r9` --
dropping `+ %r10`.  `q*d + r == n` compiled to `q*d == n`, so the program
still ran and printed plausible numbers; it surfaced only as 341 failures in
the div/mod-by-constant verification test at -O1.

Exit status: 0 when every dump assembles and none matches the pattern,
1 when a culprit is named, 2 on setup failure.

Usage:
    scripts/peephole_trace_bisect.py --lccc target/fastbuild/lccc \\
        --opt -O1 tests/regression/lea_chain_index_compose.c
    scripts/peephole_trace_bisect.py --lccc ... -O1 src.c --pattern 'my_regex'
    scripts/peephole_trace_bisect.py --trace-dir /tmp/tr      # reuse a trace
"""
from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile

PAREN = re.compile(r"\(([^()]*)\)")
VALID_SCALES = {"1", "2", "4", "8"}


def unencodable_operands(line: str) -> list[str]:
    """Return the parenthesised addressing fields of `line` that have no
    encoding.

    An x86-64 memory operand is ``disp(base,index,scale)``: at most one base
    register, at most one index register, and a scale of 1/2/4/8.  Anything
    with two registers in the index position -- e.g. ``0(,%r11,4, %r10, 1)``
    -- or more than three fields in total, cannot be encoded.

    Parsed rather than regex-matched on purpose: a first attempt with a single
    pattern missed the empty-base form ``(,%r11,4, %r10, 1)`` entirely, which
    is precisely the shape the historical bug produced.
    """
    out = []
    # `#` starts a comment in this dialect; a parenthesised comment is not an
    # operand.
    code = line.split("#", 1)[0]
    for inner in PAREN.findall(code):
        fields = [f.strip() for f in inner.split(",")]
        regs = [f for f in fields if f.startswith("%")]
        if len(regs) > 2 or len(fields) > 3:
            out.append(inner.strip())
            continue
        # Scale, when present, must be legal.  (`(%rax,%rbx)` is scale 1.)
        if len(fields) == 3 and fields[2] and fields[2] not in VALID_SCALES:
            out.append(inner.strip())
    return out

# `003-p1-fold_lea_into_load.s`
DUMP_NAME = re.compile(r"^(\d+)-p(\d+)-(.+)\.s$")


# --------------------------------------------------------------------------
# Self-test.  The operand parser is the part that decides whether a dump is
# reported, so it is the part that must not be wrong: a parser that misses the
# empty-base form would have missed the historical bug entirely (a first
# regex-based attempt did exactly that).  `scripts/peephole_trace_bisect.py
# --selftest` is wired into CI as `tests/regression/check_peephole_trace_bisect.sh`.
# --------------------------------------------------------------------------
SELFTEST_CASES: list[tuple[str, bool, str]] = [
    ("    leaq 0(,%r11,4, %r10, 1), %r9", True, "historical broken fold"),
    ("    leaq 0(,%r11,8, %r10, 1), %r9", True, "scale-8 variant"),
    ("    leaq 0(,%r10,1, %r11, 4), %r9", True, "empty base slot"),
    ("    leaq 0(,%r11,4, %r10, 1), (%r9)", True, "two bad operands"),
    ("    leaq (%rax,%rbx,3), %rcx", True, "illegal scale 3"),
    ("    leaq (%rax,%rbx,%rcx), %rdx", True, "three register fields"),
    ("    leaq (%r10, %r11, 4), %r9", False, "the fix"),
    ("    leaq (%r8, %r11, 1), %r9", False, "correct base+index"),
    ("    leaq 0(,%r11,4), %r8", False, "correct index-only"),
    ("    leaq (%rax), %rbx", False, "correct base-only"),
    ("    movl $2, 8(%rsp, %r8)", False, "correct scale-1 store"),
    ("    movq (%rcx,%rbp,8), %r13", False, "correct gather"),
    ("    movq %rax, table+8(%rcx, %rbp, 8)", False, "symbolic base+index"),
    ("    leaq .Lstr0(%rip), %rdi", False, "rip-relative"),
    ("    leaq (%rax, %rbx, 2), %rcx", False, "two regs, one scaled"),
    ("    xorl %edx, %edx  # (,%r11,4, %r10, 1)", False, "bad text in a comment"),
    ("    movq (%rdi), %rax", False, "plain load"),
]


def selftest() -> int:
    bad = 0
    for text, want, why in SELFTEST_CASES:
        got = bool(unencodable_operands(text))
        if got != want:
            bad += 1
            print(f"  FAIL want={want} got={got}  {why}: {text.strip()}")
    # Dump-name parsing: the pass name in the filename is the tool's answer.
    for name, exp in [
        ("003-p1-fold_lea_into_load.s", (3, 1, "fold_lea_into_load")),
        ("012-p0-combined_local_pass.s", (12, 0, "combined_local_pass")),
        ("000-p0-a.b.s", (0, 0, "a.b")),
    ]:
        m = DUMP_NAME.match(name)
        got = (int(m.group(1)), int(m.group(2)), m.group(3)) if m else None
        if got != exp:
            bad += 1
            print(f"  FAIL dump name {name}: want {exp}, got {got}")
    # A non-dump file must be ignored, not crash.
    if DUMP_NAME.match("notes.txt") is not None:
        bad += 1
        print("  FAIL notes.txt parsed as a dump")
    total = len(SELFTEST_CASES) + 4
    if bad:
        print(f"peephole_trace_bisect selftest: {bad} of {total} FAILED")
        return 1
    print(f"peephole_trace_bisect selftest: all {total} cases pass")
    return 0


def parse_args(argv: list[str]) -> argparse.Namespace:
    p = argparse.ArgumentParser(
        description="Find the peephole sub-pass that corrupted the assembly.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="Exit 0 when clean, 1 when a culprit pass is named, 2 on setup failure.",
    )
    p.add_argument("source", nargs="?", help="C source to compile")
    p.add_argument(
        "--lccc",
        default="target/fastbuild/lccc",
        help="lccc binary to trace (default: %(default)s)",
    )
    p.add_argument("--opt", default="-O1", help="optimization flag (default: %(default)s)")
    p.add_argument("-o", "--output", help="program output path (default: temp)")
    p.add_argument("extra", nargs="*", help="extra compiler flags")
    p.add_argument(
        "--trace-dir",
        help="reuse an existing trace directory instead of compiling",
    )
    p.add_argument(
        "--keep",
        action="store_true",
        help="keep the trace directory and print its path",
    )
    p.add_argument(
        "--pattern",
        default=None,
        help="regex reported as a corruption (default: the two-index memory operand)",
    )
    p.add_argument(
        "--assembler",
        default="as --64",
        help="assembler for the dynamic check, empty string to disable (default: %(default)s)",
    )
    p.add_argument(
        "--selftest",
        action="store_true",
        help="run the operand-parser self-test and exit (no compiler needed)",
    )
    p.add_argument(
        "--quiet",
        action="store_true",
        help="only print the verdict",
    )
    return p.parse_args(argv)


def run(cmd: list[str], **kw) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def collect_dumps(trace_dir: str) -> list[tuple[int, int, str, str]]:
    """Return (seq, iteration, pass_name, path) sorted by seq."""
    out = []
    for name in os.listdir(trace_dir):
        m = DUMP_NAME.match(name)
        if not m:
            continue
        out.append((int(m.group(1)), int(m.group(2)), m.group(3), os.path.join(trace_dir, name)))
    out.sort(key=lambda t: t[0])
    return out


def check_dump(
    path: str, user_pattern: re.Pattern | None, assembler: str
) -> tuple[str | None, list[str]]:
    """Return (failure_kind, offending_lines) for one dump."""
    text = open(path, encoding="utf-8", errors="replace").read()

    hits = []
    for ln in text.splitlines():
        if user_pattern is not None:
            if user_pattern.search(ln):
                hits.append(ln.strip())
        else:
            for field in unencodable_operands(ln):
                hits.append(f"{ln.strip()}   [({field}) has no encoding]")

    if assembler:
        as_cmd = assembler.split()
        asm_out = path + ".as.s"
        # The dumps are function fragments, so give the assembler a file it
        # can accept regardless of whether the fragment opens a section.
        body = text if ".text" in text else ".text\n" + text
        with open(asm_out, "w", encoding="utf-8") as f:
            f.write(body)
        r = run(as_cmd + ["-o", "/dev/null", asm_out])
        os.unlink(asm_out)
        if r.returncode != 0:
            errs = [ln for ln in (r.stderr or "").splitlines() if "Error" in ln or "error" in ln]
            return "does not assemble", (errs or [r.stderr.strip()])[:4]

    if hits:
        return "unencodable operand", hits[:4]
    return None, []


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    if args.selftest:
        return selftest()
    lccc = os.path.abspath(args.lccc)
    if not args.trace_dir:
        if not args.source:
            print("error: a source file or --trace-dir is required", file=sys.stderr)
            return 2
        if not os.path.exists(lccc):
            print(f"error: lccc not found at {lccc}", file=sys.stderr)
            return 2

    trace_dir = args.trace_dir
    tmp_root = None
    try:
        if not trace_dir:
            tmp_root = tempfile.mkdtemp(prefix="peephole-trace-")
            trace_dir = tmp_root
            out = args.output or os.path.join(tmp_root, "a.out")
            cmd = [lccc, args.opt, *args.extra, args.source, "-o", out]
            env = dict(os.environ, CCC_PEEPHOLE_TRACE=trace_dir)
            r = run(cmd, env=env)
            if not os.path.exists(out):
                print("error: compile produced no output; the trace is incomplete.", file=sys.stderr)
                print((r.stderr or r.stdout)[-2000:], file=sys.stderr)
                return 2

        dumps = collect_dumps(trace_dir)
        if not dumps:
            print(
                "error: no dumps written.  CCC_PEEPHOLE_TRACE must reach the "
                "compiler process, and the function must reach the peephole "
                "driver (a -O0 build with no peephole run writes nothing).",
                file=sys.stderr,
            )
            return 2

        user_pattern = re.compile(args.pattern) if args.pattern else None

        if not args.quiet:
            print(f"trace: {len(dumps)} dumps in {trace_dir}")

        first_bad = None
        for seq, it, name, path in dumps:
            kind, lines = check_dump(path, user_pattern, args.assembler)
            if not args.quiet:
                status = "ok" if kind is None else kind.upper()
                print(f"  {seq:03d} p{it} {name:<32} {status}")
            if kind and first_bad is None:
                first_bad = (seq, it, name, path, kind, lines)
                # Keep scanning so --quiet still prints a complete picture,
                # but the FIRST offender is the culprit: every later dump is
                # its downstream consequence.

        if not args.quiet:
            print()
        if first_bad is None:
            print("verdict: no dump introduces an unencodable operand; all assemble.")
            # `--keep` only matters when something was written to keep.
            return 0

        seq, it, name, path, kind, lines = first_bad
        print(f"verdict: pass `{name}` (iteration {it}) is the first offender -- {kind}.")
        print(f"         first bad dump: {path}")
        for ln in lines:
            print(f"         | {ln.strip()}")
        if args.keep:
            print(f"         trace kept in {trace_dir}")
        return 1
    finally:
        # The trace is only interesting when a culprit was named and the caller
        # asked to inspect it; otherwise it is a few hundred temp files.
        if tmp_root and not args.keep:
            shutil.rmtree(tmp_root, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
