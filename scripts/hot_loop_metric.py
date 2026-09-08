#!/usr/bin/env python3
"""Hot-loop instruction density for elementwise kernels.

Why this exists
---------------
`codegen_oracle.py` reports a STATIC instruction count for the whole
function.  For a loop kernel that number is actively misleading: a compiler
that refuses to vectorize emits one tight scalar loop and "wins", while a
compiler that vectorizes pays for a prologue, a packed body, a scalar
remainder and often an alignment peel -- more static instructions for far
less work per element.  Ranking byte/word map kernels by static size
therefore rewards exactly the codegen we are trying to beat.

The meaningful figure is how many instructions the machine retires per
ELEMENT of input in steady state:

    density = instructions in the innermost loop body / bytes processed per trip

This tool recovers both terms directly from the assembly, with no source
annotations and no compiler cooperation:

* the innermost loop body is the basic block ending in a BACKWARD branch to
  a label inside itself, choosing the block with the highest trip weight
  (the shortest backward span, i.e. the tightest cycle);
* the bytes per trip come from the pointer advance -- the immediate of the
  `add`/`sub`/`lea` that updates the induction register the memory operands
  are indexed by -- falling back to the widest vector move in the body times
  the number of loads, which is exact for unrolled unit-stride streams.

Usage:
    hot_loop_metric.py FILE.s [FILE.s ...] --symbol NAME [--json OUT]
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

# Directives and comments are not instructions.
_SKIP = re.compile(r"^\s*(?:[.#]|//|/\*|$)")
_LABEL = re.compile(r"^\s*(\.?[\w$.]+)\s*:")
_INSN = re.compile(r"^\s*([a-z][a-z0-9.]*)\s*(.*)$", re.I)
_BRANCH = re.compile(r"^j[a-z]+$", re.I)
# `addq $32, %r11` / `add $0x20,%r11` / `subq $-128, %rax`
_ADD_IMM = re.compile(r"^\$(-?(?:0x)?[0-9a-fA-F]+)\s*,\s*%(\w+)$")
# `leaq 32(%rsi), %rsi`
_LEA_SELF = re.compile(r"^(-?\d+)\(%(\w+)\)\s*,\s*%(\w+)$")

_VEC_WIDTH = {"zmm": 64, "ymm": 32, "xmm": 16}


def _int(tok: str) -> int:
    tok = tok.strip()
    neg = tok.startswith("-")
    if neg:
        tok = tok[1:]
    v = int(tok, 16) if tok.lower().startswith("0x") else int(tok)
    return -v if neg else v


def extract_function(lines: list[str], symbol: str) -> list[str]:
    """Lines belonging to `symbol`, from its label to the next symbol/size."""
    out: list[str] = []
    inside = False
    for line in lines:
        m = _LABEL.match(line)
        if m and m.group(1) == symbol:
            inside = True
            continue
        if inside:
            if re.match(rf"^\s*\.size\s+{re.escape(symbol)}\b", line):
                break
            # A new global function label ends the region.
            if m and not m.group(1).startswith("."):
                break
            out.append(line)
    return out


def all_loops(body: list[str]) -> list[tuple[list[str], str]]:
    """Every backward-branch cycle in `body`, innermost-first."""
    label_at: dict[str, int] = {}
    for idx, line in enumerate(body):
        m = _LABEL.match(line)
        if m:
            label_at[m.group(1)] = idx
    found: list[tuple[int, int, int, str]] = []
    for idx, line in enumerate(body):
        m = _INSN.match(line.strip())
        if not m or not _BRANCH.match(m.group(1)):
            continue
        target = m.group(2).strip().split()[0].rstrip(",") if m.group(2) else ""
        start = label_at.get(target)
        if start is None or start > idx:
            continue  # forward branch: not a loop backedge
        found.append((idx - start, start, idx, target))
    found.sort()
    return [(body[st : en + 1], lbl) for _, st, en, lbl in found]


def steady_state_loop(body: list[str]) -> tuple[list[str], str] | None:
    """The loop that dominates run time for a unit-stride kernel.

    NOT the tightest cycle.  A vectorized kernel contains at least two
    backedges -- the packed body and the scalar remainder -- and the
    remainder is usually the SHORTER one, so "innermost/tightest" reliably
    picks the loop that runs fewer than VF times in total and reports the
    vectorizing compiler as if it had stayed scalar.

    The steady-state loop is the one that consumes the MOST input per trip:
    the remainder is bounded by one vector width of elements regardless of
    n, while the packed body's trip count grows with n.  Ties (an unrolled
    body split across two backedges) go to the shorter body.
    """
    best: tuple[int, int, list[str], str] | None = None
    for block, label in all_loops(body):
        n, insns = count_instructions(block)
        step, _ = bytes_per_trip(insns)
        if step <= 0:
            continue
        key = (-step, n)
        if best is None or key < (best[0], best[1]):
            best = (-step, n, block, label)
    if best is not None:
        return best[2], best[3]
    # No loop had a recoverable step: fall back to the tightest cycle so the
    # caller still sees the instruction count.
    loops = all_loops(body)
    return loops[0] if loops else None


def count_instructions(block: list[str]) -> tuple[int, list[str]]:
    insns = []
    for line in block:
        s = line.strip()
        if _SKIP.match(s) or _LABEL.match(s):
            continue
        m = _INSN.match(s)
        if m:
            insns.append(s)
    return len(insns), insns


def bytes_per_trip(insns: list[str]) -> tuple[int, str]:
    """Bytes of input consumed per loop trip, with the evidence used."""
    # Registers that appear as the index/base of a memory operand.
    addressed: set[str] = set()
    for s in insns:
        for reg in re.findall(r"\(%(\w+)(?:\s*,\s*%(\w+))?", s):
            addressed.update(r for r in reg if r)

    # 1) pointer/index advance by an immediate
    candidates: list[int] = []
    for s in insns:
        m = _INSN.match(s)
        if not m:
            continue
        mnem, ops = m.group(1).lower(), m.group(2).strip()
        if mnem.startswith(("add", "sub")):
            am = _ADD_IMM.match(ops)
            if am and am.group(2) in addressed:
                step = _int(am.group(1))
                if mnem.startswith("sub"):
                    step = -step
                if step > 1:
                    candidates.append(step)
        elif mnem.startswith("lea"):
            lm = _LEA_SELF.match(ops)
            if lm and lm.group(2) == lm.group(3) and lm.group(2) in addressed:
                step = int(lm.group(1))
                if step > 1:
                    candidates.append(step)
    if candidates:
        return max(candidates), "pointer advance"

    # 2) fall back to the vector load width x number of source loads
    width = 0
    loads = 0
    for s in insns:
        for tag, w in _VEC_WIDTH.items():
            if f"%{tag}" in s:
                width = max(width, w)
        if re.match(r"^\s*v?mov[a-z]*\s+[^,]*\(", s) or re.match(r"^\s*movz", s):
            loads += 1
    if width and loads:
        return width, "vector load width"
    if loads:
        return 1, "scalar load"
    return 0, "unknown"


def analyse(path: Path, symbol: str) -> dict:
    lines = path.read_text(errors="replace").splitlines()
    body = extract_function(lines, symbol)
    if not body:
        return {"file": str(path), "error": f"symbol {symbol} not found"}
    loop = steady_state_loop(body)
    if loop is None:
        return {"file": str(path), "error": "no innermost loop found"}
    block, label = loop
    n, insns = count_instructions(block)
    step, how = bytes_per_trip(insns)
    vec = max(
        (w for s in insns for tag, w in _VEC_WIDTH.items() if f"%{tag}" in s),
        default=0,
    )
    return {
        "file": str(path),
        "symbol": symbol,
        "loop_label": label,
        "insns": n,
        "bytes_per_trip": step,
        "insns_per_byte": round(n / step, 4) if step else None,
        "widest_vector_bits": vec * 8,
        "step_evidence": how,
        "body": insns,
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("files", nargs="+", type=Path)
    ap.add_argument("--symbol", required=True)
    ap.add_argument("--json", type=Path)
    ap.add_argument("--show-body", action="store_true")
    args = ap.parse_args()

    results = [analyse(f, args.symbol) for f in args.files]
    width = max(len(Path(r["file"]).stem) for r in results)
    print(f"{'compiler':<{width}}  {'insns':>5} {'B/trip':>6} {'insn/B':>7} {'vec':>5}")
    print("-" * (width + 28))
    ranked = [r for r in results if r.get("insns_per_byte") is not None]
    best = min((r["insns_per_byte"] for r in ranked), default=None)
    for r in results:
        name = Path(r["file"]).stem
        if "error" in r:
            print(f"{name:<{width}}  {r['error']}")
            continue
        if r["insns_per_byte"] is None:
            # The step recovery failed: report the raw facts rather than
            # inventing a density (a wrong density is worse than none).
            print(f"{name:<{width}}  {r['insns']:>5} {'?':>6} {'n/a':>7} "
                  f"{r['widest_vector_bits'] or 0:>4}b  (step: {r['step_evidence']})")
            continue
        mark = "  <-- best" if best is not None and r["insns_per_byte"] == best else ""
        print(f"{name:<{width}}  {r['insns']:>5} {r['bytes_per_trip']:>6} "
              f"{r['insns_per_byte']:>7.4f} {r['widest_vector_bits'] or 0:>4}b{mark}")
        if args.show_body:
            for s in r["body"]:
                print(f"      {s}")
    if args.json:
        args.json.write_text(json.dumps(results, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
