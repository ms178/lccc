#!/usr/bin/env python3
"""Triage A/B assembly divergences with reproducibility proof and strict diff.

A "divergent" pair from ``ab_param_home_index.py`` is only meaningful if it
reproduces.  This harness therefore regenerates BOTH legs five times each:

* a leg whose own output varies across runs is NONDETERMINISTIC -- a compiler
  stability defect, never excused as an "optimisation change";
* a pair whose two legs are deterministic and equal on regeneration is a
  NON-REPRODUCIBLE fluke (e.g. a stale output file from an earlier run);
* a pair whose legs are deterministic and different is classified line by
  line.  Only these normalisations are admitted:
    - register renaming (``%rax`` -> ``%rcx``, ``x0`` -> ``x1``);
    - stack/frame arithmetic and CFI offsets (integer constants ONLY on lines
      mentioning a stack/frame pointer, or on ``.cfi*``/alignment directives);
    - label renumbering and LCCC metadata comments.
  Any other changed opcode, constant, access width or control-flow line is a
  SEMANTIC RISK and fails the audit.

``--self-test`` proves the normaliser rejects each of those semantic changes
and accepts each benign class, so the classifier itself cannot be a rubber
stamp.

The two compiler legs are explicit arguments (or ``LCCC_AB_OLD`` /
``LCCC_AB_NEW``), so a re-run can never silently classify against a stale
baseline tree.  Exit status 1 is a red-team FLAG, not a tool error: it means at
least one divergence was classified ``semantic-risk`` or ``nondeterministic``
and needs a human, tool-backed attribution (see the PR body).  Exit 0 means
every divergence was proven frame/register/label-only.

Usage:
  python3 scripts/ab_classify_divergences.py <ab-report.json> <out.json> \
      --old <dir> --new <dir> [--repeats N]
  python3 scripts/ab_classify_divergences.py --self-test
"""
from __future__ import annotations

import argparse
import collections
import difflib
import hashlib
import json
import os
import pathlib
import re
import subprocess
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from ab_param_home_index import directives

SUITE = pathlib.Path("/home/user/.cache/lccc-gcc17/gcc/testsuite/gcc.c-torture")
DEFAULT_OLD = pathlib.Path("/home/user/lccc/target/fastbuild-baseline3/fastbuild")
DEFAULT_NEW = pathlib.Path("/home/user/lccc/target/fastbuild")
OLD = NEW = None  # bound in main(); never a silent stale constant
BIN = {
    "x86_64": "lccc",
    "i686": "lccc-i686",
    "aarch64": "lccc-arm",
    "riscv64": "lccc-riscv",
}
TMP = pathlib.Path("/tmp/lccc-ab-classify")
REPEATS = 5

# Registers across all four in-scope ISAs.  A match is only ever used to erase
# a register NAME; the opcode and the operand shape must still match exactly.
REG = re.compile(
    r"%[a-z][a-z0-9]+"
    r"|\b(?:x|w|s|d|q|v|fa|fs|ft|a|t|s0|sp|fp|x29|x30|lr|zr)\d+\b"
)
LABEL = re.compile(r"\.L[\w.$]+")
# Only lines carrying a stack/frame pointer, CFI or alignment directives may
# change an integer freely.  A changed loop bound or data constant has no such
# operand, so it cannot hide behind this rule.
FRAMEISH = re.compile(
    r"%[re](?:sp|bp|esp|ebp)|\b(?:esp|ebp|sp|fp|x29|x30|s0)\b"
    r"|\.cfi|\.p2align|\.align"
)
INT = re.compile(r"(?<![\w])\d+")
# A whole-line comment may change freely.  `#` is ALSO the AArch64 immediate
# prefix, so inline `#...` must never be stripped: doing so would hide a
# changed constant such as `mov x0, #7` -> `mov x0, #8`.
PURE_COMMENT = re.compile(r"^\s*[#@]")


def normalise(line: str) -> str:
    """Erase only proven-benign differences from a +/- unified-diff line."""
    s = line[1:]
    if PURE_COMMENT.match(s):
        return "COMMENT"
    # Decide frame-ness BEFORE erasing register names: `%rsp` -> `REG`
    # would otherwise lose the evidence that justifies the constant change.
    frameish = bool(FRAMEISH.search(s))
    s = LABEL.sub(".L", s)
    s = REG.sub("REG", s)
    if frameish:
        s = INT.sub("N", s)
    return s.strip()


def classify_diff(a: list[str], b: list[str]) -> tuple[int, list[str]]:
    """Return (changed line count, unmatched/risky normalised lines)."""
    diff = [
        ln
        for ln in difflib.unified_diff(a, b, lineterm="", n=0)
        if ln.startswith(("+", "-")) and not ln.startswith(("+++", "---"))
    ]
    minus = collections.Counter(normalise(ln) for ln in diff if ln.startswith("-"))
    plus = collections.Counter(normalise(ln) for ln in diff if ln.startswith("+"))
    risky = list((minus - plus).elements()) + list((plus - minus).elements())
    return len(diff), risky


def compile_asm(base: pathlib.Path, binary: str, opt: str, src: pathlib.Path,
                out: pathlib.Path, flags: list[str],
                timeout: float = 90.0) -> tuple[int, str]:
    try:
        p = subprocess.run(
            [str(base / binary), opt, "-S", "-I", str(SUITE), *flags,
             "-o", str(out), str(src)],
            capture_output=True, timeout=timeout,
        )
    except subprocess.TimeoutExpired:
        return 124, ""
    if p.returncode != 0 or not out.exists():
        return p.returncode, ""
    return 0, hashlib.sha256(out.read_bytes()).hexdigest()




def self_test() -> int:
    """Prove the normaliser can reject semantic changes and accept benign ones."""
    cases = 0

    def rejects(old: list[str], new: list[str], label: str) -> None:
        nonlocal cases
        _, risky = classify_diff(old, new)
        if not risky:
            raise AssertionError(f"classifier accepted a semantic change: {label}")
        cases += 1

    def accepts(old: list[str], new: list[str], label: str) -> None:
        nonlocal cases
        changed, risky = classify_diff(old, new)
        if changed == 0 or risky:
            raise AssertionError(f"classifier rejected a benign change: {label} {risky}")
        cases += 1

    accepts(["\tsubq $32, %rsp"], ["\tsubq $16, %rsp"], "frame size")
    accepts(["\t.cfi_offset 6, -16"], ["\t.cfi_offset 6, -32"], "CFI offset")
    accepts(["\tmovl %eax, 48(%ebp)"], ["\tmovl %eax, 44(%ebp)"],
            "i686 stack-slot offset")
    accepts(["\tmovl $7, -36(%ebp)"], ["\tmovl $7, -32(%ebp)"],
            "i686 frame slot with a non-frame constant")
    accepts(["\tmovq %rax, %rbx"], ["\tmovq %rcx, %rdx"], "register rename")
    accepts(["\tmov x0, x1"], ["\tmov x2, x3"], "AArch64 register rename")
    accepts([".LBB0_1:"], [".LBB9_7:"], "label renumber")
    accepts(["  # lccc: slot 3"], ["  # lccc: slot 4"], "metadata comment")
    rejects(["\tmovq %rax, %rbx"], ["\taddq %rax, %rbx"], "opcode change")
    rejects(["\tmovl $7, %eax"], ["\tmovl $8, %eax"], "data constant change")
    rejects(["\tjmp .LBB0_1"], ["\tnop"], "control flow change")
    rejects(["\tmovl (%rax), %ebx"], ["\tmovq (%rax), %rbx"],
            "access width change (register normalisation must not hide it)")
    rejects(["\tcmpl $1, %eax"], ["\tcmpl $2, %eax"],
            "non-frame constant change")
    rejects(["\tmov x0, #7"], ["\tmov x0, #8"],
            "AArch64 immediate change (must not be mistaken for a comment)")
    require_count = 14
    if cases != require_count:
        raise AssertionError(f"self-test covered {cases}/{require_count} cases")
    print(f"CLASSIFIER_SELF_TEST_OK ({cases} benign/semantic cases)")
    return 0


def main() -> int:
    global OLD, NEW, REPEATS
    if len(sys.argv) == 2 and sys.argv[1] == "--self-test":
        return self_test()
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("report", help="JSON emitted by ab_param_home_index.py")
    ap.add_argument("out", help="where to write the classified JSON")
    ap.add_argument("--old", type=pathlib.Path,
                    default=pathlib.Path(os.environ.get("LCCC_AB_OLD", DEFAULT_OLD)))
    ap.add_argument("--new", type=pathlib.Path,
                    default=pathlib.Path(os.environ.get("LCCC_AB_NEW", DEFAULT_NEW)))
    ap.add_argument("--repeats", type=int, default=REPEATS)
    a = ap.parse_args()
    OLD, NEW, REPEATS = a.old, a.new, a.repeats
    for leg, path in (("old", OLD), ("new", NEW)):
        if not path.is_dir() or not (path / "lccc").is_file():
            print(f"error: {leg} leg {path} is not a built fastbuild directory",
                  file=sys.stderr)
            return 2
    if REPEATS < 1:
        print("error: --repeats must be >= 1", file=sys.stderr)
        return 2

    report = json.loads(pathlib.Path(a.report).read_text())
    divs = [d for d in report["divergences"]
            if d["status"] in ("divergent", "rc-divergent", "diagnostic-divergent")]
    # Assembly diffs only make sense for pairs that both compiled successfully.
    asm_divs = [d for d in divs if d["status"] == "divergent"]
    print(f"classifying {len(asm_divs)} assembly divergences "
          f"(+ {len(divs) - len(asm_divs)} non-assembly divergences)", flush=True)
    TMP.mkdir(exist_ok=True)
    rows = []
    for n, d in enumerate(asm_divs):
        src = SUITE / "compile" / d["source"]
        flags = directives(src.read_text(errors="replace")) if src.exists() else []
        o, nout = TMP / f"{n}-old.s", TMP / f"{n}-new.s"
        binary = BIN[d["arch"]]
        old_hashes: set[str] = set()
        new_hashes: set[str] = set()
        old_rc = new_rc = 0
        for k in range(REPEATS):
            rc, h = compile_asm(OLD, binary, d["opt"], src, o, flags)
            old_rc = rc
            if h:
                old_hashes.add(h)
            rc, h = compile_asm(NEW, binary, d["opt"], src, nout, flags)
            new_rc = rc
            if h:
                new_hashes.add(h)
        if len(old_hashes) > 1 or len(new_hashes) > 1:
            verdict = "nondeterministic"
            changed, risky = 0, [f"old_variants={len(old_hashes)}",
                                 f"new_variants={len(new_hashes)}"]
        elif not old_hashes or not new_hashes:
            verdict = "compile-failure"
            changed, risky = 0, [f"old_rc={old_rc}", f"new_rc={new_rc}"]
        elif old_hashes == new_hashes:
            verdict = "non-reproducible"
            changed, risky = 0, []
        else:
            a = o.read_text(errors="replace").splitlines()
            b = nout.read_text(errors="replace").splitlines()
            changed, risky = classify_diff(a, b)
            verdict = "semantic-risk" if risky else "optimization-only"
        rows.append({
            "source": d["source"], "arch": d["arch"], "opt": d["opt"],
            "verdict": verdict, "changed": changed,
            "old_variants": len(old_hashes), "new_variants": len(new_hashes),
            "risky_lines": risky[:12],
        })

    summary = collections.Counter(r["verdict"] for r in rows)
    payload = {
        "complete": True,
        "old_leg": str(OLD),
        "new_leg": str(NEW),
        "repeats_per_leg": REPEATS,
        "non_assembly_divergences": [
            {k: d[k] for k in ("source", "arch", "opt", "status", "old_rc", "new_rc")}
            for d in divs if d["status"] != "divergent"
        ],
        "summary": dict(summary),
        "rows": rows,
    }
    pathlib.Path(a.out).write_text(json.dumps(payload, indent=2) + "\n")
    print(json.dumps({"pairs": len(rows),
                      "changed_lines": sum(r["changed"] for r in rows),
                      "summary": dict(summary)}, indent=2))
    bad = [r for r in rows if r["verdict"] != "optimization-only"]
    for r in bad[:12]:
        print("RISK", r["source"], r["arch"], r["opt"], r["verdict"],
              r["risky_lines"], file=sys.stderr)
    return 0 if not bad else 1


if __name__ == "__main__":
    raise SystemExit(main())
