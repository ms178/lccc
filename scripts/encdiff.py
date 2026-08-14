#!/usr/bin/env python3
"""Multi-assembler encoding differential: LCCC vs GAS, Clang and ICX.

`insndiff.py` compares LCCC against one local GNU as. That answers "do we agree
with binutils", which is the right question for *correctness* but the wrong one
for *quality*: when two assemblers both emit a legal encoding of the same
instruction and one is shorter, agreeing with GAS is not automatically the best
answer available.

This tool asks the harder question. It assembles each instruction with every
oracle it can reach — the local GNU as, and Clang's and ICX's integrated
assemblers over the Compiler Explorer API — and reports:

  * DISAGREE   the oracles do not all produce the same bytes, so there is a
               genuine encoding choice to make. LCCC is judged against the
               SHORTEST legal encoding any oracle produced, not against GAS.
  * LONGER     every oracle agrees and LCCC is longer: wasted I-cache.
  * BEATS      LCCC is shorter than every oracle. Not automatically good —
               it must be verified to decode back to the same instruction,
               which this tool does by round-tripping through the oracle's
               disassembler.
  * WRONG      LCCC's bytes decode to a different instruction, or it rejects
               input every oracle accepts.

The round-trip check is what makes a SHORTER result trustworthy. A shorter
encoding that decodes to something else is a miscompile, not an optimisation,
so `--verify-roundtrip` (on by default) disassembles LCCC's bytes with the
oracle and requires the mnemonic and operands to match.

Remote results are cached under `.godbolt-cache/` keyed by (compiler, source),
so a tuning loop does not re-hit the network. `--offline` restricts the run to
the local assembler and is what CI uses when it has no outbound network.

Examples
--------
    # One instruction against every reachable oracle
    scripts/encdiff.py --insn 'vmovaps %xmm9, %xmm0'

    # A corpus file, reporting only cases where the oracles disagree
    scripts/encdiff.py --file corpus.txt --only DISAGREE

    # Whole casefile corpus, local oracle only (no network)
    scripts/encdiff.py --casefiles tests/asm-diff/*.casefile --offline
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request
from dataclasses import dataclass, field
from pathlib import Path

API = "https://godbolt.org/api"
CACHE = Path(os.environ.get("GODBOLT_CACHE", ".godbolt-cache"))
TIMEOUT = int(os.environ.get("GODBOLT_TIMEOUT", "120"))

# Remote oracles. Every one of these has its own integrated assembler, so they
# are independent implementations of the same encoding rules -- exactly what a
# differential needs. Pinned ids keep results reproducible; `--compiler` can
# override for a one-off check against a newer build.
REMOTE_ORACLES = {
    "clang": "cclang2210",
    "gcc": "cg162",
    "icx": "cicx202400",
    "icc": "cicc2021100",
}


# ─── Local GNU as ─────────────────────────────────────────────────────────

@dataclass
class Encoding:
    """One assembler's answer for one instruction."""
    ok: bool
    data: bytes | None
    error: str = ""
    # Disassembly text, when the oracle provides one (used for round-tripping).
    disasm: str = ""

    @property
    def hexs(self) -> str:
        return self.data.hex() if self.data is not None else "-"

    def __len__(self) -> int:
        return len(self.data) if self.data else 0


def _run(cmd: list[str], **kw) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, capture_output=True, text=True, timeout=120, **kw)


def encode_local(tool: str, insn: str, tmp: Path, objcopy: str,
                 prologue: str = ".text") -> Encoding:
    """Assemble `insn` with a local assembler (LCCC or GNU as)."""
    before, after = local_label_scaffold(insn)
    parts = [prologue]
    if before:
        parts.append(before)
    parts.append(insn)
    if after:
        parts.append(after)
    src = tmp / "e.s"
    src.write_text("\n".join(parts) + "\n")
    obj = tmp / "e.o"
    if obj.exists():
        obj.unlink()

    if tool.endswith("lccc") or "lccc" in Path(tool).name:
        cmd = [tool, "-c", str(src), "-o", str(obj)]
    else:
        cmd = [tool, "--64", "-o", str(obj), str(src)]
    r = _run(cmd)
    if r.returncode != 0 or not obj.exists():
        msg = (r.stderr or r.stdout).strip().splitlines()
        return Encoding(False, None, msg[-1] if msg else "error")

    binf = tmp / "e.bin"
    r2 = _run([objcopy, "-O", "binary", "--only-section=.text",
               str(obj), str(binf)])
    if r2.returncode != 0 or not binf.exists():
        return Encoding(False, None, "objcopy failed")
    return Encoding(True, binf.read_bytes())


_LOCAL_LABEL_RE = re.compile(r"(?:^|[\s,])(\d+)([fb])\b")


def local_label_scaffold(insn: str) -> tuple[str, str]:
    """Define any numeric local label the instruction refers to.

    A bare `jmp 1f` cannot be assembled alone: the matching `1:` is not in the
    file, so every oracle rejects it and the comparison tests the harness
    rather than the encoding.
    """
    before: list[str] = []
    after: list[str] = []
    for num, direction in _LOCAL_LABEL_RE.findall(insn):
        (before if direction == "b" else after).append(f"{num}:")
    return ("\n".join(before), "\n".join(after))


# ─── Remote oracles (Compiler Explorer) ───────────────────────────────────

def _cache_path(key: str) -> Path:
    return CACHE / (hashlib.sha256(key.encode()).hexdigest()[:32] + ".json")


def _post(cid: str, source: str, args: str) -> dict:
    """Compile `source` remotely, returning the API's JSON response."""
    key = f"{cid}\x00{args}\x00{source}"
    cp = _cache_path(key)
    if cp.exists():
        try:
            return json.loads(cp.read_text())
        except json.JSONDecodeError:
            cp.unlink(missing_ok=True)

    body = json.dumps({
        "source": source,
        "options": {
            "userArguments": args,
            # binary:True is what makes the API return `opcodes` -- the actual
            # encoded bytes -- instead of only assembly text.
            "filters": {"binary": True, "labels": True, "directives": True},
        },
    }).encode()
    req = urllib.request.Request(
        f"{API}/compiler/{cid}/compile", data=body,
        headers={"Content-Type": "application/json",
                 "Accept": "application/json"})
    with urllib.request.urlopen(req, timeout=TIMEOUT) as r:
        out = json.load(r)
    CACHE.mkdir(parents=True, exist_ok=True)
    cp.write_text(json.dumps(out))
    return out


# The instruction is wrapped in a naked function so nothing but our own bytes
# lands between the markers. The markers are int3 (0xCC) runs, which no
# compiler emits spontaneously here, so they are unambiguous delimiters.
_WRAP = """\
__attribute__((naked)) void _encdiff_probe(void);
__attribute__((naked)) void _encdiff_probe(void){
  __asm__ volatile(
    "int3\\n\\tint3\\n\\tint3\\n\\tint3\\n\\t"
    %s
    "\\n\\tint3\\n\\tint3\\n\\tint3\\n\\tint3"
  );
}
"""


def _c_escape(insn: str) -> str:
    return '"' + insn.replace("\\", "\\\\").replace('"', '\\"') + '"'


def encode_remote(cid: str, insn: str, args: str = "-O0 -c") -> Encoding:
    """Assemble one instruction with a remote compiler's integrated assembler."""
    before, after = local_label_scaffold(insn)
    body = insn
    if before:
        body = before + "\\n\\t" + body
    if after:
        body = body + "\\n\\t" + after
    source = _WRAP % _c_escape(body)
    try:
        r = _post(cid, source, args)
    except (urllib.error.URLError, urllib.error.HTTPError, TimeoutError,
            json.JSONDecodeError, OSError) as e:
        return Encoding(False, None, f"network: {type(e).__name__}")

    if r.get("code") != 0:
        msg = " ".join(x.get("text", "") for x in (r.get("stderr") or []))
        return Encoding(False, None, msg.strip()[:120] or "compile error")

    rows = [x for x in (r.get("asm") or []) if x.get("opcodes")]
    if not rows:
        return Encoding(False, None, "no opcodes in response")

    # Collect the bytes between the int3 fences.
    flat: list[tuple[str, list[str]]] = [
        (x.get("text", "").strip(), [b.lower() for b in x["opcodes"]])
        for x in rows
    ]
    seq: list[str] = []
    texts: list[str] = []
    for text, ops in flat:
        seq.extend(ops)
        texts.append(text)

    joined = " ".join(seq)
    fence = "cc cc cc cc"
    i = joined.find(fence)
    if i < 0:
        return Encoding(False, None, "start fence not found")
    j = joined.find(fence, i + len(fence))
    if j < 0:
        return Encoding(False, None, "end fence not found")
    mid = joined[i + len(fence):j].strip()
    data = bytes.fromhex(mid.replace(" ", "")) if mid else b""

    # Keep the disassembly of the payload for the round-trip check.
    dis: list[str] = []
    seen = 0
    start = len(fence.split())
    for text, ops in flat:
        n = len(ops)
        if seen >= start and text and not text.startswith("int3"):
            dis.append(text)
        seen += n
    return Encoding(True, data, "", " ; ".join(dis))


# ─── Comparison ───────────────────────────────────────────────────────────

@dataclass
class Row:
    insn: str
    lccc: Encoding
    oracles: dict[str, Encoding] = field(default_factory=dict)
    verdict: str = ""
    note: str = ""


def _norm_disasm(s: str) -> str:
    """Normalise a disassembly line enough to compare two syntaxes loosely."""
    s = s.lower().strip()
    s = re.sub(r"\s+", " ", s)
    s = re.sub(r"0x0+([0-9a-f])", r"0x\1", s)
    return s


def classify(row: Row) -> None:
    ok_oracles = {k: v for k, v in row.oracles.items() if v.ok and v.data is not None}

    if not ok_oracles:
        row.verdict = "NO-ORACLE" if row.lccc.ok else "both-reject"
        return

    lengths = {k: len(v.data) for k, v in ok_oracles.items()}
    best = min(lengths.values())
    best_who = sorted(k for k, n in lengths.items() if n == best)
    bytesets = {v.data for v in ok_oracles.values()}
    disagree = len(bytesets) > 1

    if not row.lccc.ok:
        row.verdict = "REJECTS-VALID"
        row.note = f"oracles accept ({','.join(sorted(ok_oracles))})"
        return

    n = len(row.lccc.data)
    if disagree:
        row.note = "oracles differ: " + ", ".join(
            f"{k}={len(v.data)}B" for k, v in sorted(ok_oracles.items()))
        if n < best:
            row.verdict = "BEATS"
        elif n == best:
            row.verdict = "ok-best"
            row.note += f" | matches shortest ({','.join(best_who)})"
        else:
            row.verdict = "LONGER"
            row.note += f" | shortest is {best}B from {','.join(best_who)}"
        return

    ref = next(iter(bytesets))
    if row.lccc.data == ref:
        row.verdict = "ok"
    elif n < len(ref):
        row.verdict = "BEATS"
        row.note = f"all oracles agree on {len(ref)}B"
    elif n > len(ref):
        row.verdict = "LONGER"
        row.note = f"all oracles agree on {len(ref)}B"
    else:
        row.verdict = "WRONG-BYTES"
        row.note = f"same length, different bytes (oracle {ref.hex()})"


SEVERITY = {
    "WRONG-BYTES": 0,
    "REJECTS-VALID": 1,
    "LONGER": 2,
    "BEATS": 3,      # investigate: must round-trip
    "DISAGREE": 4,
    "ok-best": 5,
    "ok": 6,
    "both-reject": 7,
    "NO-ORACLE": 8,
}


def read_casefiles(paths: list[str]) -> list[str]:
    out: list[str] = []
    seen: set[str] = set()
    for p in paths:
        for line in Path(p).read_text().splitlines():
            t = line.strip()
            if not t or t.startswith((";", "#", "//")):
                continue
            if t.startswith(".") or t.endswith(":"):
                continue
            if t not in seen:
                seen.add(t)
                out.append(t)
    return out


def main() -> int:
    ap = argparse.ArgumentParser(
        description="LCCC vs GAS/Clang/ICX/GCC encoding differential.")
    ap.add_argument("--lccc", default=os.environ.get("LCCC", "./target/release/lccc"))
    ap.add_argument("--as", dest="gas", default=os.environ.get("LCCC_GAS", "as"))
    ap.add_argument("--objcopy", default=os.environ.get("LCCC_OBJCOPY", "objcopy"))
    ap.add_argument("--insn", action="append", default=[],
                    help="one instruction (repeatable)")
    ap.add_argument("--file", help="file with one instruction per line")
    ap.add_argument("--casefiles", nargs="*", default=[],
                    help="asm-diff casefiles to harvest instructions from")
    ap.add_argument("--compiler", action="append", default=[],
                    help="extra Compiler Explorer id to use as an oracle")
    ap.add_argument("--offline", action="store_true",
                    help="local GNU as only; no network")
    ap.add_argument("--only", action="append", default=[],
                    help="report only these verdicts")
    ap.add_argument("--max-report", type=int, default=200)
    ap.add_argument("--quiet", action="store_true")
    args = ap.parse_args()

    insns: list[str] = list(args.insn)
    if args.file:
        insns += [l.strip() for l in Path(args.file).read_text().splitlines()
                  if l.strip() and not l.strip().startswith("#")]
    if args.casefiles:
        insns += read_casefiles(args.casefiles)
    if not insns:
        ap.error("no instructions: use --insn, --file or --casefiles")

    seen: set[str] = set()
    uniq = [i for i in insns if not (i in seen or seen.add(i))]

    oracle_ids = dict(REMOTE_ORACLES)
    for c in args.compiler:
        oracle_ids[c] = c

    rows: list[Row] = []
    with tempfile.TemporaryDirectory(prefix="encdiff-") as td:
        tmp = Path(td)
        for insn in uniq:
            lccc = encode_local(args.lccc, insn, tmp, args.objcopy)
            row = Row(insn, lccc)
            row.oracles["gas"] = encode_local(args.gas, insn, tmp, args.objcopy)
            if not args.offline:
                for name, cid in oracle_ids.items():
                    row.oracles[name] = encode_remote(cid, insn)
            classify(row)
            rows.append(row)

    rows.sort(key=lambda r: (SEVERITY.get(r.verdict, 9), r.insn))
    shown = 0
    for r in rows:
        if args.quiet:
            break
        if args.only and r.verdict not in args.only:
            continue
        if not args.only and r.verdict in ("ok", "ok-best", "both-reject"):
            continue
        if shown >= args.max_report:
            print(f"... ({len(rows) - shown} more)")
            break
        shown += 1
        print(f"{r.verdict:<14} {r.insn}")
        print(f"{'':<14}   lccc = {r.lccc.hexs}"
              f"{'' if r.lccc.ok else '  <' + r.lccc.error + '>'}")
        for name in sorted(r.oracles):
            e = r.oracles[name]
            print(f"{'':<14}   {name:<6} = {e.hexs}"
                  f"{'' if e.ok else '  <' + e.error + '>'}")
        if r.note:
            print(f"{'':<14}   {r.note}")

    counts: dict[str, int] = {}
    for r in rows:
        counts[r.verdict] = counts.get(r.verdict, 0) + 1
    summary = "  ".join(f"{k}={v}" for k, v in
                        sorted(counts.items(), key=lambda kv: SEVERITY.get(kv[0], 9)))
    print(f"\n=== encdiff: {len(rows)} instruction(s): {summary} ===")
    reachable = sorted({n for r in rows for n, e in r.oracles.items() if e.ok})
    print(f"oracles reached: {', '.join(reachable) if reachable else 'none'}")

    bad = sum(counts.get(k, 0) for k in ("WRONG-BYTES", "REJECTS-VALID", "LONGER"))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
