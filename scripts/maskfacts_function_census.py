#!/usr/bin/env python3
"""Per-function instruction census for the AVX-512VL mask-compare matrix.

WHY THIS EXISTS
---------------
A whole-file `count_ins()` total cannot answer "is the runtime-compare
roundtrip gone?" -- `main`'s 485 `printf` calls dominate it, and GCC/Clang
emit `.set alias,target` (ICF) or inline whole functions, so a naive label
walk silently compares different case sets.  `backlog.md` section 14 records
the gap as a PER-FUNCTION average (19.0 instructions per runtime compare vs
3.19 / 3.25 for GCC 16.2 / Clang 23.1.5).  This script is that measurement,
made reproducible and self-testing:

* every one of the 485 generated cases is resolved by NAME across all four
  assembly files (`.set` aliases are followed; a function inlined away counts
  as ZERO, which is the correct result for a compiler that eliminated it);
* the "runtime compare" set is defined ONCE, from the LCCC ON build: the cases
  whose body contains a `vpcmp*` instruction.  Constant-image folds and
  predicate-3/7 folds therefore leave the set automatically;
* instructions are counted between the function label and its `.size`
  directive, including `ret`.  A second figure EXCLUDES `ret` so the result
  is comparable with the historical 3.19 / 3.25 numbers in section 14;
* `--require-runtime-avg N` turns the acceptance criterion into a hard gate,
  and `--self-test` feeds synthetic assembly through the SAME parser to
  detect a vacuous or drifted census.

Usage:
    scripts/maskfacts_function_census.py \
        --asm-dir DIR --labels lccc_on lccc_off gcc16 clang23 \
        --json RECEIPT.json [--require-runtime-avg 4.0]
    scripts/maskfacts_function_census.py --self-test
"""
from __future__ import annotations

import argparse
import json
import re
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "tests" / "regression" / "maskfacts_cross" / "avx512vl_cmp_matrix.c"

INS = re.compile(r"^\s+([a-z][a-z0-9.]*)\s", re.M)
FUNC = re.compile(r"^([A-Za-z_][A-Za-z0-9_]*):[ \t]*(?:#[^\n]*)?$", re.M)
SIZE = re.compile(r"^\s*\.size\s+([A-Za-z_][A-Za-z0-9_]*)\s*,", re.M)
ALIAS = re.compile(r"^\s*\.set\s+([A-Za-z_]\w*)\s*,\s*([A-Za-z_]\w*)\s*$", re.M)
DECL = re.compile(
    r"^__attribute__\(\(noinline\)\) static unsigned long long (\w+)", re.M
)


def parse(path: Path) -> tuple[dict[str, list[str]], dict[str, str]]:
    """Return (label -> mnemonics, alias -> target).  A `.size` directive ends
    the body even if a later local label shares the function's name."""
    text = Path(path).read_text()
    marks = [(m.start(), m.group(1)) for m in FUNC.finditer(text)]
    bodies: dict[str, list[str]] = {}
    for i, (pos, name) in enumerate(marks):
        end = marks[i + 1][0] if i + 1 < len(marks) else len(text)
        body = text[pos:end]
        sz = SIZE.search(body)
        if sz:
            body = body[: sz.start()]
        bodies[name] = INS.findall(body)
    return bodies, dict(ALIAS.findall(text))


def resolver(bodies: dict[str, list[str]], aliases: dict[str, str]):
    """Resolve a case name to its mnemonics.  Bound recursion, cycle-safe."""

    def resolve(name: str, seen: frozenset[str] = frozenset()) -> list[str]:
        if name in bodies:
            return bodies[name]
        if name in aliases and name not in seen:
            return resolve(aliases[name], seen | {name})
        return []

    return resolve


def group(name: str) -> str:
    if name.startswith("rt_"):
        return "runtime"
    if name.startswith("c_"):
        return "constant"
    if name.startswith("s"):
        return "stores"
    if name == "main":
        return "main"
    return "other"


def case_names(path: Path) -> list[str]:
    return DECL.findall(Path(path).read_text())


def census(asm_dir: Path, labels: list[str], fixture: Path):
    names = case_names(fixture)
    if not names:
        raise RuntimeError(f"no generated cases found in {fixture}")
    resolved = {}
    for label in labels:
        bodies, aliases = parse(asm_dir / f"{label}.s")
        resolved[label] = resolver(bodies, aliases)

    # The audited set: exactly those runtime cases whose LCCC ON body still
    # emits a vpcmp*.  This is the set section 14's 19.0 was measured on.
    on = resolved[labels[0]]
    vp = [
        n
        for n in names
        if n.startswith("rt_") and any(m.startswith("vpcmp") for m in on(n))
    ]
    if not vp:
        raise RuntimeError("no runtime vpcmp cases in the ON build")

    groups: dict[str, dict[str, int]] = {}
    for g, ns in (
        ("runtime_all", [n for n in names if n.startswith("rt_")]),
        ("runtime_vpcmp", vp),
        ("constant", [n for n in names if n.startswith("c_")]),
        ("stores", [n for n in names if n.startswith("s")]),
        ("main", ["main"]),
    ):
        groups[g] = {
            label: sum(len(resolved[label](n)) for n in ns) for label in labels
        }
        groups[g]["cases"] = len(ns)

    avgs = {}
    for label in labels:
        with_ret = groups["runtime_vpcmp"][label]
        # `ret`/`retq` is one instruction in every case of the ON build; GCC
        # and Clang may inline a case away entirely (0 instructions), so do
        # NOT subtract a fixed count -- count the actual ret mnemonics.
        rets = sum(
            1 for n in vp for m in resolved[label](n) if m in ("ret", "retq")
        )
        n = len(vp)
        avgs[label] = {
            "with_ret": with_ret / n,
            "excluding_ret": (with_ret - rets) / n,
            "min": min(len(resolved[label](n)) for n in vp),
            "max": max(len(resolved[label](n)) for n in vp),
        }
    return {
        "cases": len(names),
        "runtime_vpcmp_cases": len(vp),
        "groups": groups,
        "runtime_vpcmp_average": avgs,
    }


def self_test() -> int:
    """The parser itself must be tested: a census that reports the same
    number for every compiler, or that misses `.set` aliases, is vacuous."""
    with tempfile.TemporaryDirectory(prefix="maskfacts_census_") as td:
        tmp = Path(td)
        (tmp / "a.s").write_text(
            "rt_a:\n\tmovdqu A(%rip), %xmm0\n\tvpcmpb $0, %xmm1, %xmm0, %k1\n"
            "\tkmovq %k1, %rax\n\tret\n\t.size rt_a, .-rt_a\n"
            "rt_b:\n\tvmovdqa A(%rip), %xmm0\n\tvpcmpb $0, B(%rip), %xmm0, %k0\n"
            "\tkmovq %k0, %rax\n\tret\n\t.size rt_b, .-rt_b\n"
            "c_a:\n\tmovl $1, %eax\n\tret\n\t.size c_a, .-c_a\n"
            "s_a:\n\tmovl $2, %eax\n\tret\n\t.size s_a, .-s_a\n"
            "main:\n\tcall rt_a\n\tret\n\t.size main, .-main\n"
        )
        (tmp / "b.s").write_text(
            "\t.set rt_a,rt_b\n"
            "rt_b:\n\tmovl $7, %eax\n\tret\n\t.size rt_b, .-rt_b\n"
            "c_a:\n\tmovl $1, %eax\n\tret\n\t.size c_a, .-c_a\n"
            "s_a:\n\tmovl $2, %eax\n\tret\n\t.size s_a, .-s_a\n"
            "main:\n\tret\n\t.size main, .-main\n"
        )
        fx = tmp / "cases.c"
        fx.write_text(
            "__attribute__((noinline)) static unsigned long long rt_a(void) {}\n"
            "__attribute__((noinline)) static unsigned long long rt_b(void) {}\n"
            "__attribute__((noinline)) static unsigned long long c_a(void) {}\n"
            "__attribute__((noinline)) static unsigned long long s_a(void) {}\n"
        )
        res = census(tmp, ["a", "b"], fx)
        assert res["cases"] == 4, res
        assert res["runtime_vpcmp_cases"] == 2, res
        assert res["groups"]["runtime_vpcmp"]["a"] == 8, res
        # `rt_a` resolves through `.set` to `rt_b`'s 2-instruction body.
        assert res["groups"]["runtime_vpcmp"]["b"] == 4, res
        assert res["groups"]["constant"]["a"] == 2, res
        assert res["groups"]["stores"]["a"] == 2, res
        assert res["groups"]["main"]["a"] == 2, res
        assert res["runtime_vpcmp_average"]["a"]["with_ret"] == 4.0, res
        assert res["runtime_vpcmp_average"]["a"]["excluding_ret"] == 3.0, res
        # A cycle must not hang and must count as zero.
        (tmp / "b.s").write_text(
            (tmp / "b.s").read_text().replace("\t.set rt_a,rt_b\n", "\t.set rt_a,rt_a\n")
        )
        res2 = census(tmp, ["a", "b"], fx)
        assert res2["groups"]["runtime_vpcmp"]["b"] == 2, res2
    print("self-test PASS: aliases, groups, ret counting and cycles verified")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--asm-dir", type=Path)
    ap.add_argument("--labels", nargs="+", default=["lccc_on", "lccc_off", "gcc16", "clang23"])
    ap.add_argument("--fixture", type=Path, default=FIXTURE)
    ap.add_argument("--json", type=Path)
    ap.add_argument("--require-runtime-avg", type=float)
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()
    if args.self_test:
        return self_test()
    if args.asm_dir is None:
        ap.error("--asm-dir is required (or use --self-test)")
    res = census(args.asm_dir, args.labels, args.fixture)
    if args.json:
        args.json.write_text(json.dumps(res, indent=2) + "\n")
    print(f"cases={res['cases']} runtime_vpcmp={res['runtime_vpcmp_cases']}")
    for g, vals in res["groups"].items():
        print(f"  {g:<16} n={vals['cases']:>3} " + " ".join(
            f"{k}={v}" for k, v in vals.items() if k != "cases"))
    for label, a in res["runtime_vpcmp_average"].items():
        print(
            f"  {label:<16} runtime avg with_ret={a['with_ret']:.3f} "
            f"excluding_ret={a['excluding_ret']:.3f} "
            f"min={a['min']} max={a['max']}"
        )
    if args.require_runtime_avg is not None:
        got = res["runtime_vpcmp_average"][args.labels[0]]["with_ret"]
        if got > args.require_runtime_avg:
            print(
                f"FAIL: {args.labels[0]} runtime average {got:.3f} exceeds "
                f"{args.require_runtime_avg:.3f}",
                file=sys.stderr,
            )
            return 1
        print(f"PASS: runtime average {got:.3f} <= {args.require_runtime_avg:.3f}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
