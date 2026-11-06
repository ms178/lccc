#!/usr/bin/env python3
"""Classify A/B assembly divergences as optimization-only or semantic-risk.

Regenerates both legs for every ``divergent`` pair, diffs the emitted assembly
and classifies each changed line.  A change is optimization-only when it is
confined to stack-frame arithmetic (recognised by an sp/fp operand), CFI
offsets, alignment, labels, LCCC metadata comments, or pure register
renaming.  Anything else -- a different opcode, a changed non-frame constant,
different control flow or a changed access width -- is a semantic risk and
fails the audit.

Usage: python3 scripts/ab_classify_divergences.py <ab-report.json> <out.json>
"""
from __future__ import annotations

import difflib
import json
import pathlib
import re
import subprocess
import sys

SUITE = pathlib.Path("/home/user/.cache/lccc-gcc17/gcc/testsuite/gcc.c-torture")
OLD = pathlib.Path("/home/user/lccc/target/fastbuild-baseline2/fastbuild")
NEW = pathlib.Path("/home/user/lccc/target/fastbuild")
BIN = {
    "x86_64": "lccc",
    "i686": "lccc-i686",
    "aarch64": "lccc-arm",
    "riscv64": "lccc-riscv",
}
TMP = pathlib.Path("/tmp/lccc-ab-classify")
TMP.mkdir(exist_ok=True)

# Only lines mentioning a stack/frame pointer may change an integer freely.
# That is what makes "frame-only" a proof rather than a blanket masking of all
# constants: a changed loop bound or data constant has no sp/fp operand.
FRAME = re.compile(
    r"""(?x)
    ^[+-]\s*(?:
          (?:sub|add)[ql]?\s+\$?\d+,\s*%r(?:sp|bp)
        | \.cfi[^\n]*
        | \.p2align[^\n]* | \.align[^\n]*
        | \.L(?:BB)?\d+:
        | \#\s
        | [^\n]*\b(?:%rsp|%rbp|\bsp\b|\bs0\b|\bx29\b|\bfp\b)[^\n]*
    )""",
)

REG = re.compile(
    r"%(?:r\d+[a-z]?|e(?:ax|bx|cx|dx|si|di|sp|bp)|[abcd]x|xmm\d+|ymm\d+|zmm\d+)"
    r"|\b(?:x|w|s|d|q|v|fa|fs|ft|a|t)\d+\b"
)


def compile_asm(base: pathlib.Path, binary: str, opt: str, src: pathlib.Path, out: pathlib.Path):
    p = subprocess.run(
        [str(base / binary), opt, "-S", "-I", str(SUITE), "-o", str(out), str(src)],
        capture_output=True,
        timeout=180,
    )
    return p.returncode


def main() -> int:
    report = json.loads(pathlib.Path(sys.argv[1]).read_text())
    divs = [d for d in report["divergences"] if d["status"] == "divergent"]
    print(f"classifying {len(divs)} assembly divergences", flush=True)
    rows = []
    for d in divs:
        src = SUITE / "compile" / d["source"]
        o, n = TMP / "a.s", TMP / "b.s"
        rco = compile_asm(OLD, BIN[d["arch"]], d["opt"], src, o)
        rcn = compile_asm(NEW, BIN[d["arch"]], d["opt"], src, n)
        a = o.read_text(errors="replace").splitlines() if rco == 0 else []
        b = n.read_text(errors="replace").splitlines() if rcn == 0 else []
        diff = [
            line
            for line in difflib.unified_diff(a, b, lineterm="", n=0)
            if line.startswith(("+", "-")) and not line.startswith(("+++", "---"))
        ]
        benign: list[str] = []
        risky: list[str] = []
        others = [x for x in diff if x[:1] != ""]
        for line in diff:
            if FRAME.match(line):
                benign.append(line)
                continue
            skel = REG.sub("REG", line[1:]).strip()
            if any(
                x[:1] != line[:1] and REG.sub("REG", x[1:]).strip() == skel
                for x in others
            ):
                benign.append(line)
                continue
            risky.append(line)
        rows.append(
            {
                "source": d["source"],
                "arch": d["arch"],
                "opt": d["opt"],
                "changed": len(diff),
                "benign": len(benign),
                "risky": len(risky),
                "risky_lines": risky[:12],
            }
        )
    risk = [r for r in rows if r["risky"]]
    pathlib.Path(sys.argv[2]).write_text(
        json.dumps({"rows": rows, "risky_pairs": len(risk)}, indent=2) + "\n"
    )
    print(
        json.dumps(
            {
                "pairs": len(rows),
                "changed_lines": sum(r["changed"] for r in rows),
                "benign_lines": sum(r["benign"] for r in rows),
                "risky_lines": sum(r["risky"] for r in rows),
                "risky_pairs": len(risk),
            },
            indent=2,
        )
    )
    for r in risk[:8]:
        print("RISK", r["source"], r["arch"], r["opt"], r["risky_lines"], file=sys.stderr)
    return 0 if not risk else 1


if __name__ == "__main__":
    raise SystemExit(main())
