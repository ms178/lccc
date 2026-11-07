#!/usr/bin/env python3
"""Inline-asm dialect alternatives must match GCC 16.2 AND Clang 23.1.5.

The rule, read back from `gcc/final.cc::output_asm_insn` and then measured on
both oracles, has TWO independent gates:

* `#ifdef ASSEMBLER_DIALECT` -- only i386 defines it (`ix86_asm_dialect`).
  `gcc/config/aarch64/aarch64.h` and `gcc/config/riscv/riscv.h` do not, so on
  those targets `{`, `}` and `|` are ordinary characters. That is what makes
  `ld1 {v0.4s}, [x0]` work on AArch64.
* extended asm only. A statement with operands, clobbers or goto labels lowers
  to ASM_OPERANDS and runs through `output_asm_insn`; a basic asm becomes
  ASM_INPUT and is emitted verbatim. `volatile` is irrelevant.

When both hold, `{...}` keeps the first (AT&T) alternative and drops the braces,
including the pipeless `{l}` of `mov{l}`. `%{`, `%}` and `%|` are the escapes.

Measured agreement (this is the point of the gate -- the two compilers were
suspected of disagreeing and do not):

    x86-64 extended  mov{l} %%eax,%%ebx -> movl %eax, %ebx   GCC yes, Clang yes
    x86-64 extended  no{p|q}            -> nop               GCC yes, Clang yes
    x86-64 extended  nop{|} / nop{}     -> nop               GCC yes, Clang yes
    x86-64 basic     mov{l} %eax,%ebx   -> verbatim          GCC yes, Clang yes
    aarch64 extended ld1 {v0.4s},[x0]   -> verbatim          Clang yes (GCC: no
                                                             dialect defined)

Clang validates asm text with its integrated assembler even for -S, so every
template here is a REAL instruction after expansion. A template that only
looks plausible would be rejected by Clang for reasons unrelated to braces.

RISC-V has no instruction whose syntax contains braces, so no oracle can
accept a brace-carrying template there. Those rows assert LCCC's text fidelity
against the rule and are labelled as rule-only rather than silently skipped.

Usage:
    tests/regression/check_asm_dialect_alternatives.py --self-test
    tests/regression/check_asm_dialect_alternatives.py \
        --lccc target/fastbuild/lccc --gcc gcc-16 --clang clang-23 --json out.json
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]

# Templates valid on x86 after expansion. `%%eax` is used instead of an operand
# reference so the comparison isolates brace handling from register allocation.
X86_SHAPES = [
    ("pipeless_suffix", "mov{l} %%eax, %%ebx", "input"),
    ("piped_suffix", "mov{l|q} %%eax, %%ebx", "input"),
    ("piped_mnemonic", "no{p|q}", "input"),
    ("pipeless_mnemonic", "no{p}", "input"),
    ("empty_first", "nop{|}", "input"),
    ("empty_group", "nop{}", "input"),
    ("two_groups", "no{p|q}{|}", "input"),
    ("escaped_braces", "nop # %{lit%}", "input"),
    ("escaped_pipe", "nop # %{a%|b%}", "input"),
    ("comment_group", "# {x|y} comment", "input"),
    ("clob_pipeless", "no{p}", "clobber"),
    ("clob_piped", "no{p|q}", "clobber"),
    ("basic_pipeless", "mov{l} %eax, %ebx", "basic"),
    ("basic_piped", "no{p|q}", "basic"),
    ("basic_volatile", "no{p}", "basic_volatile"),
]

# Valid on AArch64 with braces intact; an expansion would break them.
A64_SHAPES = [
    ("neon_list_clobber", "ld1 {v0.4s}, [x0]", "clobber"),
    ("neon_list_basic", "ld1 {v0.4s}, [x0]", "basic"),
    # The input is deliberately unreferenced: comparing `%0`'s register would
    # test register allocation, not brace handling (LCCC stages x9 where Clang
    # keeps x0 -- both correct).
    ("neon_list_input", "ld1 {v0.4s}, [x0]", "input"),
]

# RISC-V: rule-only text fidelity (no oracle can accept braces here).
RISCV_SHAPES = [
    ("brace_literal", "nop # {a|b}", "clobber"),
    ("brace_pipeless", "nop # {solo}", "clobber"),
    ("escape_literal", "nop # %{lit%}", "clobber"),
]


def source_for(template: str, kind: str) -> str:
    t = json.dumps(template)
    if kind == "input":
        return f"long f(long a) {{ __asm__ ({t} : : \"r\"(a)); return a; }}\n"
    if kind == "clobber":
        return f'void f(void) {{ __asm__ ({t} ::: "memory"); }}\n'
    if kind == "basic_volatile":
        return f"void f(void) {{ __asm__ volatile ({t}); }}\n"
    return f"void f(void) {{ __asm__ ({t}); }}\n"


FENCE = re.compile(r"^(?:#|//|;)\s*(NO_)?APP\s*$")
BOOKKEEP = re.compile(r"^#\s+\d+\s+")


def app_lines(assembly: str) -> list[str]:
    """The user's template text. Handles GCC's `#APP`, Clang's `#APP` on x86 and
    `//APP` on AArch64/RISC-V, and drops GCC's `# <line> "<file>" 1` bookkeeping.
    Clang's asm printer respaces `{v0.4s}` as `{ v0.4s }`, so whitespace around
    brace interiors is normalised; everything else is compared exactly."""
    out: list[str] = []
    inside = False
    for raw in assembly.splitlines():
        s = raw.strip()
        m = FENCE.match(s)
        if m:
            inside = m.group(1) is None
            continue
        if not inside or BOOKKEEP.match(s) or not s:
            continue
        s = re.sub(r"\{\s*", "{", s)
        s = re.sub(r"\s*\}", "}", s)
        s = re.sub(r"\s+", " ", s)
        out.append(s)
    return out


def expand(template: str, extended: bool, dialects: bool) -> str:
    """The rule, implemented independently of the compiler under test."""
    if not dialects or not extended:
        return template
    out: list[str] = []
    i = 0
    in_group = False
    while i < len(template):
        c = template[i]
        if c == "%" and i + 1 < len(template):
            n = template[i + 1]
            if n in "{}|":
                out.append(n)
                i += 2
                continue
            if n == "%":
                out.append("%%")
                i += 2
                continue
        if c == "{":
            if in_group:
                return template
            in_group = True
            i += 1
        elif c == "}":
            if not in_group:
                out.append("}")
            in_group = False
            i += 1
        elif c == "|" and in_group:
            i += 1
            closed = False
            while i < len(template):
                if template[i] == "%" and i + 1 < len(template):
                    i += 2
                    continue
                if template[i] == "}":
                    closed = True
                    i += 1
                    break
                i += 1
            if not closed:
                return template
            in_group = False
        else:
            out.append(c)
            i += 1
    return template if in_group else "".join(out)


def normalize(text: str) -> str:
    text = re.sub(r"\{\s*", "{", text)
    text = re.sub(r"\s*\}", "}", text)
    return re.sub(r"\s+", " ", text).strip()


def expected(template: str, kind: str, dialects: bool) -> str:
    extended = kind not in ("basic", "basic_volatile")
    text = expand(template, extended, dialects)
    # `%%` collapses to `%` in extended asm; basic asm is verbatim.
    if extended:
        text = text.replace("%%", "%")
    return normalize(text)


def run(cmd: list[str], timeout: int) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, capture_output=True, timeout=timeout)


def self_test() -> int:
    checks = [
        ("gcc fence", app_lines("#APP\n\tnop\n#NO_APP\n") == ["nop"]),
        ("clang aarch64 fence", app_lines("//APP\n\tld1 { v0.4s }, [x0]\n//NO_APP\n")
         == ["ld1 {v0.4s}, [x0]"]),
        ("bookkeeping dropped",
         app_lines('#APP\n# 3 "t.c" 1\n\tnop\n# 0 "" 2\n#NO_APP\n') == ["nop"]),
        ("outside ignored", app_lines("\tmovl $1, %eax\n") == []),
        ("pipeless strips on x86", expected("mov{l} %%eax", "input", True) == "movl %eax"),
        # `%%` collapsing belongs to operand substitution, not to dialects,
        # so it still happens on a target with no dialect alternatives.
        ("pipeless literal without dialects",
         expected("mov{l} %%eax", "input", False) == "mov{l} %eax"),
        ("basic verbatim", expected("no{p|q}", "basic", True) == "no{p|q}"),
        ("clobber is extended", expected("no{p|q}", "clobber", True) == "nop"),
        ("empty first alt", expected("nop{|}", "input", True) == "nop"),
        ("empty group", expected("nop{}", "input", True) == "nop"),
        ("escapes", expected("nop # %{a%|b%}", "input", True) == "nop # {a|b}"),
        ("neon literal on aarch64",
         expected("ld1 {v0.4s}, [x0]", "clobber", False) == "ld1 {v0.4s}, [x0]"),
        ("nested is passed through", expand("{a{b|c}|d}", True, True) == "{a{b|c}|d}"),
        ("unterminated is passed through", expand("{a|", True, True) == "{a|"),
    ]
    bad = [n for n, ok in checks if not ok]
    for n, ok in checks:
        print(f"  {'ok  ' if ok else 'FAIL'} {n}")
    if bad:
        print(f"self-test FAILED: {bad}", file=sys.stderr)
        return 1
    print("self-test: extraction and the dialect rule agree")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--lccc", default=str(ROOT / "target/fastbuild/lccc"))
    ap.add_argument("--lccc-i686")
    ap.add_argument("--lccc-arm")
    ap.add_argument("--lccc-riscv")
    ap.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"))
    ap.add_argument("--clang", default=os.environ.get("CLANG", "clang-23"))
    ap.add_argument("--timeout", type=int, default=120)
    ap.add_argument("--json", type=Path)
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()
    if args.self_test:
        return self_test()

    host = Path(args.lccc)
    gcc, clang = shutil.which(args.gcc), shutil.which(args.clang)
    # (target, compiler, extra flags, dialects, shapes, oracle command or None)
    legs = [("x86_64", host, [], True, X86_SHAPES,
             [gcc, "-O2", "-S"] if gcc else None)]
    for name, explicit, fn, dialects, shapes, oracle in (
        ("i686", args.lccc_i686, "lccc-i686", True, X86_SHAPES,
         [gcc, "-m32", "-O2", "-S"] if gcc else None),
        ("aarch64", args.lccc_arm, "lccc-arm", False, A64_SHAPES,
         [clang, "--target=aarch64-linux-gnu", "-O2", "-S"] if clang else None),
        ("riscv64", args.lccc_riscv, "lccc-riscv", False, RISCV_SHAPES, None),
    ):
        cand = Path(explicit) if explicit else host.with_name(fn)
        if cand.is_file():
            legs.append((name, cand, [], dialects, shapes, oracle))
        else:
            print(f"SKIP {name}: {cand} not built")

    report: dict = {"rows": [], "targets": [leg[0] for leg in legs],
                    "rule_only": [], "oracle_skipped": []}
    failures: list[str] = []
    with tempfile.TemporaryDirectory() as tmp:
        d = Path(tmp)
        for target, compiler, extra, dialects, shapes, oracle in legs:
            for name, template, kind in shapes:
                src = d / f"{target}.{name}.c"
                src.write_text(source_for(template, kind))
                want = expected(template, kind, dialects)
                asm = d / f"{target}.{name}.s"
                got = run([str(compiler), *extra, "-O2", "-S", str(src), "-o", str(asm)],
                          args.timeout)
                row = {"target": target, "shape": name, "kind": kind,
                       "expected": want, "rc": got.returncode}
                if got.returncode != 0:
                    row["error"] = got.stderr.decode(errors="replace").strip()[:200]
                    failures.append(f"{target}/{name}: -S failed: {row['error']}")
                else:
                    actual = app_lines(asm.read_text(errors="replace"))
                    row["actual"] = actual
                    if actual != ([want] if want else []):
                        failures.append(
                            f"{target}/{name}: {actual!r} != expected {[want]!r}")
                report["rows"].append(row)
                if oracle is None:
                    report["rule_only"].append(f"{target}/{name}")
                    continue
                # Clang validates the template; a rejection is itself data.
                oasm = d / f"{target}.{name}.oracle.s"
                o = run([*oracle, str(src), "-o", str(oasm)], args.timeout)
                orow = {"target": target, "shape": name, "rc": o.returncode}
                if o.returncode != 0:
                    orow["error"] = o.stderr.decode(errors="replace").strip()[:200]
                    # GCC never assembles for -S, so it must always accept.
                    if oracle[0] == gcc:
                        failures.append(f"oracle/{target}/{name}: {orow['error']}")
                    else:
                        report["oracle_skipped"].append(
                            f"{target}/{name}: {orow['error']}")
                else:
                    oactual = app_lines(oasm.read_text(errors="replace"))
                    orow["actual"] = oactual
                    if oactual != ([want] if want else []):
                        failures.append(
                            f"oracle/{target}/{name}: compilers disagree: "
                            f"{oactual!r} != {[want]!r}")
                report["rows"].append(orow | {"oracle": True})

    report["failures"] = failures
    report["correct"] = not failures
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps(report, indent=2) + "\n")
    for f in failures:
        print(f"FAIL {f}", file=sys.stderr)
    if failures:
        print(f"FAIL asm dialect alternatives: {len(failures)} problems", file=sys.stderr)
        return 1
    print(f"PASS asm dialect alternatives: {len(report['rows'])} rows on "
          f"{', '.join(report['targets'])}; {len(report['rule_only'])} rule-only "
          f"(no oracle syntax), {len(report['oracle_skipped'])} oracle rejections recorded")
    return 0


if __name__ == "__main__":
    sys.exit(main())
