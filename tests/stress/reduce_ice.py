#!/usr/bin/env python3
"""Structure-aware ICE/CRASH reducer for stress-lab programs.

``run_stress.py`` reduces MISMATCHes for free (each case is an independent
function and is re-emitted alone), but an ICE or a runtime CRASH gives no
failing *case*: the compiler died somewhere inside the whole translation
unit.  This tool shrinks such a program to the smallest set of cases that
still reproduces the failure, using the structural knowledge the generator
has instead of a generic line-based delta debugger:

* ``main`` is a sequence of independent ``{ ... }`` check blocks, one per
  case; a block can be deleted without touching any other block.
* Every case function is ``static``; once no block references it, the
  function (and its ``static const`` probe tables) is dead and is deleted as
  well, so the final reproducer contains only the cases that matter.

The oracle is "the failure signature is still present": for an ICE the
compiler exits non-zero and the first ``internal error`` / ``panicked`` line
has the same head; for a CRASH the produced binary dies by the same signal.
ddmin over blocks (halving granularity, then singles) converges in
O(k log n) compiler invocations for k culprit blocks.

Usage:
  tests/stress/reduce_ice.py <program.c> --lccc target/fastbuild/lccc \\
      --flags -O1 [--out reduced.c] [--mode ice|crash] [--timeout 60]

Exit status: 0 when a reduced reproducer was written, 2 when the input does
not reproduce the failure with the given flags.
"""
from __future__ import annotations

import argparse
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

SIG_RE = re.compile(r"(internal error.*|panicked at.*|error:.*|Segmentation fault.*)")


def split_main(src: str) -> tuple[str, list[str], str]:
    """Return (prefix-up-to-first-block, blocks, suffix-after-last-block)."""
    m = re.search(r"^int main\(void\) \{\n    int fails = 0;\n", src, re.M)
    if not m:
        raise SystemExit("reduce_ice: input is not a stress-lab program (no main header)")
    body_start = m.end()
    tail = src.index("    if (fails == 0) puts", body_start)
    body = src[body_start:tail]
    blocks = [p for p in re.split(r"(?m)^(?=    \{ )", body) if p.strip()]
    return src[:body_start], blocks, src[tail:]


def strip_dead_cases(prefix: str, blocks: list[str]) -> str:
    """Delete case functions/tables not referenced by any surviving block."""
    live: set[str] = set()
    for b in blocks:
        live.update(re.findall(r"\b([a-z]+_\d+)\(", b))
    # Case functions are <family>_<n>; helpers share the numeric suffix
    # (sw_fn_11, sw_probes_11, ...) — keep every chunk with a live suffix.
    suffixes = {name.rsplit("_", 1)[1] for name in live}
    out: list[str] = []
    for chunk in re.split(r"\n(?=(?:static|typedef|struct|union|#|int main))", prefix):
        names = re.findall(r"\b[A-Za-z_]+_(\d+)\b", chunk)
        if chunk.startswith(("int main", "#")) or not names or any(n in suffixes for n in names):
            out.append(chunk)
    return "\n".join(out)


def signature(text: str) -> str:
    m = SIG_RE.search(text)
    return m.group(1).strip()[:160] if m else ""


class Oracle:
    def __init__(self, lccc: str, flags: list[str], mode: str, timeout: int, work: Path):
        self.lccc, self.flags, self.mode, self.timeout, self.work = lccc, flags, mode, timeout, work
        self.expected: str | None = None
        self.runs = 0

    def probe(self, src: str) -> str | None:
        """Return the failure signature for `src`, or None when it is fine."""
        self.runs += 1
        c = self.work / f"cand{self.runs}.c"
        exe = self.work / f"cand{self.runs}"
        c.write_text(src)
        try:
            cp = subprocess.run([self.lccc, *self.flags, str(c), "-o", str(exe)],
                                capture_output=True, text=True, timeout=self.timeout)
        except subprocess.TimeoutExpired:
            return "TIMEOUT(compile)"
        if self.mode == "ice":
            if cp.returncode == 0:
                return None
            return signature(cp.stderr) or f"exit {cp.returncode}"
        if cp.returncode != 0:
            return None  # a different failure; not the crash we are chasing
        try:
            rp = subprocess.run([str(exe)], capture_output=True, text=True, timeout=self.timeout)
        except subprocess.TimeoutExpired:
            return "TIMEOUT(run)"
        return f"signal {-rp.returncode}" if rp.returncode < 0 else None

    def fails(self, src: str) -> bool:
        sig = self.probe(src)
        if sig is None:
            return False
        if self.expected is None:
            self.expected = sig
            return True
        # Same failure class: value ids / register numbers differ between
        # candidates, so compare only the head of the signature.
        return sig.split(":")[0] == self.expected.split(":")[0]


def ddmin(blocks: list[str], test) -> list[str]:
    n = 2
    while len(blocks) >= 2:
        chunk = max(1, len(blocks) // n)
        reduced = False
        for i in range(0, len(blocks), chunk):
            cand = blocks[:i] + blocks[i + chunk:]
            if cand and test(cand):
                blocks, n, reduced = cand, max(n - 1, 2), True
                break
        if not reduced:
            if n >= len(blocks):
                break
            n = min(n * 2, len(blocks))
    return blocks


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("program")
    ap.add_argument("--lccc", default=str(Path(__file__).resolve().parents[2] / "target/fastbuild/lccc"))
    ap.add_argument("--flags", nargs="*", default=["-O1"])
    ap.add_argument("--mode", choices=["ice", "crash"], default="ice")
    ap.add_argument("--timeout", type=int, default=60)
    ap.add_argument("--out")
    a = ap.parse_args()

    src = Path(a.program).read_text()
    prefix, blocks, suffix = split_main(src)
    work = Path(tempfile.mkdtemp(prefix="lccc-reduce-"))
    orc = Oracle(a.lccc, a.flags, a.mode, a.timeout, work)

    def assemble(bs: list[str]) -> str:
        return strip_dead_cases(prefix, bs) + "".join(bs) + suffix

    if not orc.fails(assemble(blocks)):
        print("reduce_ice: input does not reproduce with", a.flags, file=sys.stderr)
        shutil.rmtree(work, ignore_errors=True)
        return 2
    print(f"reduce_ice: signature = {orc.expected}", file=sys.stderr)
    print(f"reduce_ice: {len(blocks)} blocks to start", file=sys.stderr)
    kept = ddmin(blocks, lambda bs: orc.fails(assemble(bs)))
    final = assemble(kept)
    out = Path(a.out) if a.out else Path(a.program).with_suffix(".reduced.c")
    out.write_text(final)
    print(f"reduce_ice: {len(kept)} block(s) remain after {orc.runs} compiles -> {out}", file=sys.stderr)
    shutil.rmtree(work, ignore_errors=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
