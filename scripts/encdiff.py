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
import concurrent.futures
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
_OBJDUMP = os.environ.get("LCCC_OBJDUMP", "objdump")
TIMEOUT = int(os.environ.get("GODBOLT_TIMEOUT", "120"))

# Remote oracles. Every one of these has its own integrated assembler, so they
# are independent implementations of the same encoding rules -- exactly what a
# differential needs. Pinned ids keep results reproducible; `--compiler` can
# override for a one-off check against a newer build.
REMOTE_ORACLES = {
    "clang": "cclang2210",
    "gcc": "cg162",
    "icx": "cicxlatest",
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
# lands between the markers. The markers are `ud2` runs: unlike int3 (0xCC),
# ud2 is never emitted as alignment padding, and it cannot be confused with a
# payload byte because fences are matched on the disassembled MNEMONIC, not on
# raw bytes -- an instruction such as `dec %r12d` (41 ff cc) ends in 0xCC and
# broke a byte-level scanner.
_WRAP = """\
__attribute__((naked)) void _encdiff_probe(void);
__attribute__((naked)) void _encdiff_probe(void){
  __asm__ volatile(
    "ud2\\n\\tud2\\n\\tud2\\n\\tud2\\n\\t"
    %s
    "\\n\\tud2\\n\\tud2\\n\\tud2\\n\\tud2"
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

    rows = [(x.get("text", "").strip(), [b.lower() for b in x["opcodes"]])
            for x in (r.get("asm") or []) if x.get("opcodes")]
    if not rows:
        return Encoding(False, None, "no opcodes in response")
    return _split_fenced(rows, 1)[0]


_BATCH_WRAP_HEAD = "__attribute__((naked)) void _p%d(void);\n" \
                   "__attribute__((naked)) void _p%d(void){ __asm__ volatile(\n"
_BATCH_WRAP_TAIL = "  );\n}\n"


def _batch_source(insns: list[str]) -> str:
    """One naked function per instruction, each inside its own int3 fence."""
    out = []
    for i, insn in enumerate(insns):
        before, after = local_label_scaffold(insn)
        body = insn
        if before:
            body = before + "\\n\\t" + body
        if after:
            body = body + "\\n\\t" + after
        out.append(_BATCH_WRAP_HEAD % (i, i))
        out.append('    "ud2\\n\\tud2\\n\\tud2\\n\\tud2\\n\\t"\n')
        out.append("    " + _c_escape(body) + "\n")
        out.append('    "\\n\\tud2\\n\\tud2\\n\\tud2\\n\\tud2"\n')
        out.append(_BATCH_WRAP_TAIL)
    return "".join(out)


def _split_fenced(rows: list[tuple[str, list[str]]], count: int) -> list[Encoding]:
    """Cut the row stream into `count` fenced payloads.

    Splitting on raw 0xCC bytes is WRONG: an instruction may legitimately end
    in 0xCC (`dec %r12d` is 41 ff cc), and a byte scanner then treats its last
    byte as the opening of the closing fence and truncates the encoding.
    Compiler Explorer reports one row per instruction, so fences are detected
    as rows whose disassembly is `int3`, which no payload row can imitate.

    Padding matters too: the compiler aligns each function, so an alignment
    NOP sits between one payload's closing fence and the next opening fence.
    Payload extraction therefore only takes rows between a fence run and the
    NEXT fence run, and anything after a closing fence is skipped until the
    following opening fence is seen.
    """
    def is_fence(text: str) -> bool:
        return text.strip().split()[0:1] == ["ud2"]

    # Collect maximal fence runs, then treat consecutive pairs as delimiters.
    runs: list[tuple[int, int]] = []   # (start_row, end_row_exclusive)
    i = 0
    n = len(rows)
    while i < n:
        if is_fence(rows[i][0]):
            j = i
            while j < n and is_fence(rows[j][0]):
                j += 1
            if j - i >= 4:
                runs.append((i, j))
            i = j
        else:
            i += 1

    out: list[Encoding] = []
    for k in range(0, len(runs) - 1, 2):
        if len(out) >= count:
            break
        _, open_end = runs[k]
        close_start, _ = runs[k + 1]
        payload = rows[open_end:close_start]
        data = bytes.fromhex("".join(b for _, ops in payload for b in ops))
        texts = [t for t, _ in payload if t]
        out.append(Encoding(True, data, "", " ; ".join(texts)))
    while len(out) < count:
        out.append(Encoding(False, None, "fence not found"))
    return out


def _split_labelled(asm_rows: list[dict], count: int) -> list[Encoding]:
    """Extract each probe's payload using the `_pN:` function labels.

    Positional fence pairing is fragile: whether the compiler emits an
    alignment NOP after a probe depends on that probe's own length, and a
    payload ending in `ret` gets none at all. The wrapper emits one labelled
    function per instruction, so slicing by label is exact and independent of
    padding.
    """
    def is_fence(text: str) -> bool:
        return text.strip().split()[0:1] == ["ud2"]

    # Bucket rows by the most recent `_pN:` label.
    buckets: dict[int, list[tuple[str, list[str]]]] = {}
    cur: int | None = None
    for x in asm_rows:
        text = (x.get("text") or "").strip()
        m = re.match(r"^_p(\d+):", text)
        if m:
            cur = int(m.group(1))
            buckets.setdefault(cur, [])
            continue
        if cur is None or not x.get("opcodes"):
            continue
        buckets[cur].append((text, [b.lower() for b in x["opcodes"]]))

    out: list[Encoding] = []
    for i in range(count):
        rows = buckets.get(i)
        if not rows:
            out.append(Encoding(False, None, "probe label not found"))
            continue
        # Drop the leading fence run, then take everything up to the next one.
        k = 0
        while k < len(rows) and is_fence(rows[k][0]):
            k += 1
        payload = []
        while k < len(rows) and not is_fence(rows[k][0]):
            payload.append(rows[k])
            k += 1
        data = bytes.fromhex("".join(b for _, ops in payload for b in ops))
        texts = [t for t, _ in payload if t]
        out.append(Encoding(True, data, "", " ; ".join(texts)))
    return out


# The fence mnemonic cannot be measured by this transport: a probe consisting
# of `ud2` is indistinguishable from its own delimiter. Route it to the local
# assembler only rather than reporting a bogus remote answer.
UNMEASURABLE_REMOTE = {"ud2"}


def encode_remote_many(cid: str, insns: list[str], args: str = "-O0 -c",
                       chunk: int = 60) -> list[Encoding]:
    """Assemble many instructions remotely, batching them into few requests.

    A batch is only usable if the whole translation unit compiles. One bad
    instruction would fail the entire chunk, so on failure the chunk is split
    and retried, isolating the offender in O(log n) extra requests instead of
    giving up on all of its neighbours.
    """
    if not insns:
        return []
    results: list[Encoding] = []
    for base in range(0, len(insns), chunk):
        group = insns[base:base + chunk]
        # Keep unmeasurable probes out of the batch entirely; they would
        # desync the fence structure for their neighbours.
        idx = [k for k, i in enumerate(group)
               if i.strip().split()[0:1] != list(UNMEASURABLE_REMOTE)]
        sendable = [group[k] for k in idx]
        got = _encode_group(cid, sendable, args) if sendable else []
        merged: list[Encoding] = [
            Encoding(False, None, "not measurable over this transport")
            for _ in group
        ]
        for pos, k in enumerate(idx):
            if pos < len(got):
                merged[k] = got[pos]
        results.extend(merged)
    return results


def _encode_group(cid: str, group: list[str], args: str) -> list[Encoding]:
    if not group:
        return []
    try:
        r = _post(cid, _batch_source(group), args)
    except (urllib.error.URLError, urllib.error.HTTPError, TimeoutError,
            json.JSONDecodeError, OSError) as e:
        return [Encoding(False, None, f"network: {type(e).__name__}")] * len(group)

    if r.get("code") != 0:
        if len(group) == 1:
            msg = " ".join(x.get("text", "") for x in (r.get("stderr") or []))
            return [Encoding(False, None, msg.strip()[:120] or "compile error")]
        mid = len(group) // 2
        return _encode_group(cid, group[:mid], args) + \
               _encode_group(cid, group[mid:], args)

    got = _split_labelled(r.get("asm") or [], len(group))
    if len(got) != len(group):
        got += [Encoding(False, None, "batch desync")] * (len(group) - len(got))
    return got


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


_SCALE1 = re.compile(r"\(,%([a-z0-9]+),1\)")
_ZERODISP = re.compile(r"(?<![0-9a-fx])0x0\(")
_COMMUTATIVE_VEX = {
    "vpand", "vpor", "vpxor", "vpaddb", "vpaddw", "vpaddd", "vpaddq",
    "vpmullw", "vpaddsb", "vpaddsw", "vpaddusb", "vpaddusw",
    "vpminub", "vpmaxub", "vpminsw", "vpmaxsw", "vpavgb", "vpavgw",
    "vpmulhw", "vpmulhuw", "vpcmpeqb", "vpcmpeqw", "vpcmpeqd",
    "vandps", "vandpd", "vorps", "vorpd", "vxorps", "vxorpd",
}
_VEX3 = re.compile(r"^(v\S+)\s+(%\S+),(%\S+),(%\S+)$")
_GP32_TO_64 = {
    "eax": "rax", "ebx": "rbx", "ecx": "rcx", "edx": "rdx",
    "esi": "rsi", "edi": "rdi", "ebp": "rbp", "esp": "rsp",
    **{f"r{n}d": f"r{n}" for n in range(8, 16)},
}
_MOV_IMM = re.compile(r"^(movabs|mov)\s+\$(0x[0-9a-f]+|\d+),%(\w+)$")


def _canon_insn(insn: str) -> str:
    """Canonicalise the spellings that differ only by encoding choice."""
    insn = insn.split("#")[0].strip().lower()
    insn = _SCALE1.sub(r"(%\1)", insn)
    insn = _ZERODISP.sub("(", insn)
    insn = re.sub(r"\s+", " ", insn)
    m = _VEX3.match(insn)
    if m and m.group(1) in _COMMUTATIVE_VEX:
        a, b = sorted((m.group(2), m.group(3)))
        insn = f"{m.group(1)} {a},{b},{m.group(4)}"
    m = _MOV_IMM.match(insn)
    if m:
        val = int(m.group(2), 0)
        reg = m.group(3)
        if reg in _GP32_TO_64 and 0 <= val <= 0xFFFFFFFF:
            insn = f"mov ${val:#x},%{_GP32_TO_64[reg]}"
    return insn


def decodes_same(objdump: str, a: bytes, b: bytes) -> bool:
    """True when two byte strings disassemble to the same instruction."""
    def dis(data: bytes) -> str:
        if not data:
            return ""
        with tempfile.TemporaryDirectory(prefix="encdiff-dis-") as td:
            raw = Path(td) / "d.bin"
            raw.write_bytes(data)
            r = subprocess.run(
                [objdump, "-D", "-b", "binary", "-m", "i386:x86-64",
                 "-M", "att", str(raw)],
                capture_output=True, text=True, timeout=120)
        out = []
        for line in r.stdout.splitlines():
            m = re.match(r"^\s+[0-9a-f]+:\s+((?:[0-9a-f]{2} )+)\s*\t(.*)$", line)
            if m:
                out.append(_canon_insn(m.group(2)))
        return "\n".join(out)
    try:
        return dis(a) == dis(b)
    except (OSError, subprocess.SubprocessError):
        return False


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
        elif is_wrong_shorter(row.insn):
            row.verdict = "DECLINED-WRONG"
            row.note += (f" | {best}B form from {','.join(best_who)} is not"
                         " equivalent; refused")
        elif is_declined_fp_swap(row.insn):
            row.verdict = "DECLINED-FP"
            row.note += (f" | {best}B form from {','.join(best_who)} needs a"
                         " source swap; refused, FP add/mul propagate SRC1's"
                         " NaN payload")
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
    elif decodes_same(_OBJDUMP, row.lccc.data, ref):
        # Same length, different bytes, but the two decode identically -- a
        # different spelling of the same instruction, not a defect.
        row.verdict = "ok"
    else:
        row.verdict = "WRONG-BYTES"
        row.note = f"same length, different bytes (oracle {ref.hex()})"


# Cases where a shorter encoding EXISTS but is deliberately not taken.
#
# clang and icx exchange the sources of vaddps/vaddpd/vmulps/vmulpd to reach
# the 2-byte VEX prefix. That is one byte smaller and, for ordinary operands,
# gives the same result -- but x86 FP add/mul are not bit-commutative: when
# both sources are NaN the result carries SRC1's payload. Measured on a
# Skylake-SP host, `vaddps` over (0x7fc00001, 0x7fc00002) yields 0x7fc00001 in
# one operand order and 0x7fc00002 in the other.
#
# Reporting these as LONGER forever would train the reader to ignore LONGER,
# so they are classified separately and explained.
_FP_NONCOMMUTATIVE = {"vaddps", "vaddpd", "vmulps", "vmulpd",
                      "vaddss", "vaddsd", "vmulss", "vmulsd"}


def is_declined_fp_swap(insn: str) -> bool:
    """True for a swap we refuse on NaN-payload grounds."""
    parts = insn.replace(",", " ").split()
    return bool(parts) and parts[0] in _FP_NONCOMMUTATIVE


# Shorter encodings that are simply WRONG. Every one of these was produced by
# an oracle and rejected here after checking what the bytes actually do.
#
#   xchg %eax,%eax -> 0x90
#       ICC emits the one-byte NOP. But `xchg %eax,%eax` is a real 32-bit
#       register write and therefore zeroes the upper half of RAX, while 0x90
#       does nothing at all. Measured: starting from RAX=0x1122334455667788,
#       `87 c0` leaves 0x0000000055667788 and `90` leaves it unchanged.
_WRONG_SHORTER = {
    "xchg %eax, %eax", "xchg %eax,%eax",
}


def is_wrong_shorter(insn: str) -> bool:
    return " ".join(insn.split()) in _WRONG_SHORTER


SEVERITY = {
    "WRONG-BYTES": 0,
    "DECLINED-FP": 6,   # shorter form exists but changes NaN payload
    "DECLINED-WRONG": 6, # shorter form exists but is not equivalent
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
    ap.add_argument("--batch", type=int, default=60,
                    help="instructions per remote request (0 disables batching)")
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
        # Local assemblers are cheap; run them straight through.
        locals_lccc = [encode_local(args.lccc, i, tmp, args.objcopy) for i in uniq]
        locals_gas = [encode_local(args.gas, i, tmp, args.objcopy) for i in uniq]

        remote: dict[str, list[Encoding]] = {}
        if not args.offline:
            # Each oracle is an independent network stream, so fetch them
            # concurrently; within an oracle the instructions are batched.
            with concurrent.futures.ThreadPoolExecutor(
                    max_workers=max(1, len(oracle_ids))) as ex:
                futs = {
                    ex.submit(encode_remote_many, cid, uniq, "-O0 -c",
                              args.batch): name
                    for name, cid in oracle_ids.items()
                }
                for fut in concurrent.futures.as_completed(futs):
                    name = futs[fut]
                    try:
                        remote[name] = fut.result()
                    except Exception as e:  # noqa: BLE001
                        remote[name] = [Encoding(False, None,
                                                 f"oracle failed: {e}")] * len(uniq)

        for k, insn in enumerate(uniq):
            row = Row(insn, locals_lccc[k])
            row.oracles["gas"] = locals_gas[k]
            for name, encs in remote.items():
                if k < len(encs):
                    row.oracles[name] = encs[k]
            classify(row)
            rows.append(row)

    rows.sort(key=lambda r: (SEVERITY.get(r.verdict, 9), r.insn))
    shown = 0
    for r in rows:
        if args.quiet:
            break
        if args.only and r.verdict not in args.only:
            continue
        if not args.only and r.verdict in ("ok", "ok-best", "both-reject",
                                           "DECLINED-FP", "DECLINED-WRONG"):
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
