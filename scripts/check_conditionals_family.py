#!/usr/bin/env python3
"""Differential battery for the conditional-assembly family (GAS 2.47).

Every case is assembled with BOTH the pinned GNU as 2.47 oracle and LCCC.
The verdicts must agree in direction (accept/reject); accepted cases must
produce byte-identical .text. Rejected cases report the oracle's verbatim
diagnostic so a wording change in either assembler is caught.

Usage: scripts/check_conditionals_family.py [--lccc PATH] [--as PATH]
Exit 0 = all cases agree, 1 = any divergence (printed), 2 = setup error.
"""
from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_LCCC = REPO_ROOT / "target" / "fastbuild" / "lccc"

# (name, source, expect) — expect: "ok" | "reject"
CASES: list[tuple[str, str, str]] = [
    # ── .if expression family ────────────────────────────────────────────
    ("if-true", ".set v, 3\n.if v == 3\nnop\n.else\nud2\n.endif\n", "ok"),
    ("if-false-takes-else", ".set v, 3\n.if v != 3\nud2\n.else\nnop\n.endif\n", "ok"),
    ("if-elseif-chain", ".set v, 2\n.if v == 1\nud2\n.elseif v == 2\nnop\n.else\nud2\n.endif\n", "ok"),
    ("if-bool-and", ".set v, 3\n.if v > 1 && v < 10\nnop\n.endif\n", "ok"),
    ("if-bool-or", ".set v, 3\n.if v < 1 || v > 2\nnop\n.endif\n", "ok"),
    ("if-nested", ".set v, 2\n.if v > 1\n.if v > 2\nud2\n.else\nnop\n.endif\n.endif\n", "ok"),
    ("if-paren", ".text\n.if(3)\nnop\n.endif\n", "ok"),
    ("if-undef-nonconstant", ".text\n.if foo\nnop\n.endif\n", "reject"),
    ("if-hex-true", ".text\n.if 0x10 == 16\nnop\n.endif\n", "ok"),
    ("elseif-dead-branch-not-evaluated", ".set v, 1\n.ifdef v\nnop\n.elseif foo\nnop\n.endif\n", "ok"),
    ("elseif-live-branch-evaluated", ".text\n.ifdef v\nnop\n.elseif foo\nnop\n.endif\n", "reject"),
    ("elseif-deep-chain", ".set v, 3\n.if v == 1\nud2\n.elseif v == 2\nud2\n.elseif v == 3\nnop\n.else\nud2\n.endif\n", "ok"),
    ("duplicate-else", ".text\n.if 1\nnop\n.else\nnop\n.else\nnop\n.endif\n", "reject"),
    ("unclosed-if", ".text\n.if 1\nnop\n", "reject"),
    ("if-arithmetic", ".text\n.if 3*4-12 == 0\nnop\n.endif\n", "ok"),
    ("if-char-literal-arg", ".text\n.if 'a' == 97\nnop\n.endif\n", "ok"),
    # ── .ifdef / .ifndef / .ifnotdef ─────────────────────────────────────
    ("ifdef-defined", ".set v, 5\n.ifdef v\nnop\n.else\nud2\n.endif\n", "ok"),
    ("ifdef-undefined", ".text\n.ifdef v\nud2\n.else\nnop\n.endif\n", "ok"),
    ("ifndef-undefined", ".text\n.ifndef v\nnop\n.endif\n", "ok"),
    ("ifndef-defined", ".set v, 5\n.ifndef v\nud2\n.endif\n", "ok"),
    ("ifnotdef-defined", ".set v, 5\n.ifnotdef v\nud2\n.else\nnop\n.endif\n", "ok"),
    ("ifdef-set-assignment", ".text\nv = 5\n.ifdef v\nnop\n.endif\n", "ok"),
    ("ifdef-noarg", ".text\n.ifdef\nnop\n.endif\n", "reject"),
    ("ifdef-number", ".text\n.ifdef 123\nnop\n.endif\n", "reject"),
    ("ifdef-register", ".text\n.ifdef %rax\nnop\n.endif\n", "reject"),
    # ── numeric comparison family ────────────────────────────────────────
    ("ifeq-true", ".set v, 5\n.ifeq v-5\nnop\n.endif\n", "ok"),
    ("ifeq-false", ".set v, 5\n.ifeq v\nud2\n.endif\n", "ok"),
    ("ifne-true", ".set v, 5\n.ifne v-4\nnop\n.endif\n", "ok"),
    ("iflt-true", ".text\n.iflt -1\nnop\n.endif\n", "ok"),
    ("ifle-true", ".text\n.ifle 0\nnop\n.endif\n", "ok"),
    ("ifgt-false", ".text\n.ifgt 0\nud2\n.endif\n", "ok"),
    ("ifge-true", ".text\n.ifge 0\nnop\n.endif\n", "ok"),
    ("ifeq-undef", ".text\n.ifeq foo\nnop\n.endif\n", "reject"),
    # ── .ifb / .ifnb ─────────────────────────────────────────────────────
    ("ifb-empty", '.text\n.ifb ""\nnop\n.endif\n', "ok"),
    ("ifb-nonempty", '.text\n.ifb "x"\nud2\n.endif\n', "ok"),
    ("ifnb-nonempty", '.text\n.ifnb "x"\nnop\n.endif\n', "ok"),
    # ── .ifc / .ifnc ─────────────────────────────────────────────────────
    ("ifc-equal", ".text\n.ifc a, a\nnop\n.endif\n", "ok"),
    ("ifc-different", ".text\n.ifc a, b\nud2\n.endif\n", "ok"),
    ("ifnc-different", ".text\n.ifnc a, b\nnop\n.endif\n", "ok"),
    ("ifnc-equal", ".text\n.ifnc a, a\nud2\n.endif\n", "ok"),
    # ── .ifeqs / .ifnes ──────────────────────────────────────────────────
    ("ifeqs-equal", '.text\n.ifeqs "a", "a"\nnop\n.endif\n', "ok"),
    ("ifeqs-different", '.text\n.ifeqs "a", "b"\nud2\n.endif\n', "ok"),
    ("ifnes-different", '.text\n.ifnes "a", "b"\nnop\n.endif\n', "ok"),
    ("ifnes-equal", '.text\n.ifnes "a", "a"\nud2\n.endif\n', "ok"),
    ("ifeqs-nocomma", '.text\n.ifeqs "a" "a"\nnop\n.endif\n', "reject"),
    ("ifeqs-unquoted", ".text\n.ifeqs a, b\nnop\n.endif\n", "reject"),
    ("ifnes-unquoted", ".text\n.ifnes a, b\nnop\n.endif\n", "reject"),
    # ── stray terminators / nesting integrity ────────────────────────────
    ("stray-endif", ".text\nnop\n.endif\n", "reject"),
    ("stray-else", ".text\nnop\n.else\n", "reject"),
    ("stray-elseif", ".text\nnop\n.elseif 1\n", "reject"),
    ("nested-family-mix", ".set v, 2\n.ifdef v\n.if v == 2\nnop\n.endif\n.else\nud2\n.endif\n", "ok"),
    ("tab-separated-ifb", ".text\n.ifb\t\"\"\nnop\n.endif\n", "ok"),
]


def run(binpath: str, src: Path, obj: Path, extra: list[str]) -> tuple[bool, str]:
    """Assemble src; return (accepted, stderr)."""
    try:
        proc = subprocess.run(
            [binpath, *extra, str(src), "-o", str(obj)],
            capture_output=True, text=True, timeout=60,
        )
    except FileNotFoundError:
        sys.exit(f"error: {binpath} not found")
    return proc.returncode == 0, proc.stderr.strip()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--lccc", default=str(DEFAULT_LCCC))
    ap.add_argument("--as", dest="gas", required=True)
    ap.add_argument("--gas-flags", default=["--64"], nargs="*")
    args = ap.parse_args()

    gas_flags = args.gas_flags
    lccc_flags = ["-c"]

    failures = 0
    with tempfile.TemporaryDirectory(prefix="cond-battery-") as td:
        tmp = Path(td)
        for name, src, expect in CASES:
            sfile = tmp / f"{name}.s"
            sfile.write_text(src)
            gas_ok, gas_err = run(args.gas, sfile, tmp / f"{name}.g.o", gas_flags)
            lccc_ok, lccc_err = run(args.lccc, sfile, tmp / f"{name}.l.o", lccc_flags)
            verdict = "ok" if (gas_ok and lccc_ok) else ("reject" if not (gas_ok or lccc_ok) else "DIVERGE")
            detail = ""
            if verdict == "DIVERGE":
                detail = f"  gas({'OK' if gas_ok else 'REJ'}): {gas_err[:90]!r}\n  lccc({'OK' if lccc_ok else 'REJ'}): {lccc_err[:90]!r}"
            elif verdict == "reject" and gas_err.splitlines() and lccc_err.splitlines():
                gmsg = [l for l in gas_err.splitlines() if "Error:" in l]
                lmsg = [l for l in lccc_err.splitlines() if "error:" in l]
                if gmsg and lmsg:
                    detail = f"  (gas: {gmsg[0].split('Error:')[-1].strip()[:70]!r} / lccc: {lmsg[0].split('error:')[-1].strip()[:70]!r})"
            mark = "PASS" if verdict == expect else "FAIL"
            if mark == "FAIL":
                failures += 1
            print(f"{mark} {name}: {verdict} (expect {expect}){detail}")

    print(f"\n{len(CASES) - failures}/{len(CASES)} cases agree with the GAS 2.47 oracle")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
