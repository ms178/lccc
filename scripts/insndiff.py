#!/usr/bin/env python3
"""Per-instruction encoding differential between LCCC and GNU as.

LCCC has its own integrated assembler. Any divergence from GNU as in the bytes
it emits is either a wrong encoding (a miscompile that only shows up at run
time) or a missed size optimization (dead bytes in the I-cache). Both matter,
and both are invisible unless the encodings are compared directly.

This tool assembles instructions ONE AT A TIME with both assemblers and
reports the byte-level difference, so an encoding bug reduces to a single
mnemonic in one step instead of a bisect over a whole object file. It also
classifies each divergence, because "LCCC emits 6 bytes where GAS emits 3" and
"LCCC emits different bytes of the same length" are very different failures.

Usage
-----
    # one-off
    echo 'addw $65535, %ax' | scripts/insndiff.py

    # a list, showing only what differs
    scripts/insndiff.py --file cases.txt --only-diff

    # sweep a template over a register/immediate matrix
    scripts/insndiff.py --sweep 'add{S} ${IMM}, %{R{S}}'

    # exit non-zero on any divergence (CI gate)
    scripts/insndiff.py --file cases.txt --quiet

Exit status is 0 when every instruction agrees, 1 when any diverges, and 2 on
a setup error (missing tool, unreadable input).
"""
from __future__ import annotations

import argparse
import itertools
import os
import re
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_LCCC = REPO_ROOT / "target" / "release" / "lccc"

# ─── Register / immediate vocabularies for --sweep ────────────────────────

VOCAB: dict[str, list[str]] = {
    "R8":  ["al", "cl", "dl", "bl", "spl", "bpl", "sil", "dil",
            "r8b", "r9b", "r12b", "r15b"],
    "R8L": ["al", "cl", "dl", "bl", "ah", "ch", "dh", "bh"],
    "R16": ["ax", "cx", "dx", "bx", "sp", "bp", "si", "di",
            "r8w", "r12w", "r15w"],
    "R32": ["eax", "ecx", "edx", "ebx", "esp", "ebp", "esi", "edi",
            "r8d", "r12d", "r13d", "r15d"],
    "R64": ["rax", "rcx", "rdx", "rbx", "rsp", "rbp", "rsi", "rdi",
            "r8", "r12", "r13", "r15"],
    "XMM": [f"xmm{i}" for i in (0, 1, 7, 8, 12, 15)],
    "YMM": [f"ymm{i}" for i in (0, 1, 7, 8, 12, 15)],
    # Structurally interesting bases: rsp/r12 force a SIB byte, rbp/r13 force
    # a displacement, r8+ force REX.B.
    "BASE": ["rax", "rcx", "rsp", "rbp", "rsi", "r8", "r12", "r13", "r15"],
    "IDX":  ["rax", "rbx", "rbp", "r12", "r13", "r15"],
    "SCALE": ["1", "2", "4", "8"],
    # Displacement and immediate boundaries: every place the encoder must
    # switch width, plus one value on each side of it.
    "DISP": ["0", "1", "-1", "127", "128", "-128", "-129", "255", "256",
             "32767", "2147483647", "-2147483648"],
    "IMM":  ["0", "1", "-1", "127", "128", "-128", "-129", "255", "256",
             "65535", "2147483647", "-2147483648"],
    "IMM8": ["0", "1", "127", "128", "255"],
    "SHIFT": ["0", "1", "7", "15", "31", "63", "255"],
    "S":    ["b", "w", "l", "q"],
    "SEG":  ["cs", "ds", "es", "fs", "gs", "ss"],
}

# Suffix letter -> register vocabulary, for the `%{R{S}}` shorthand.
SUFFIX_REGS = {"b": "R8", "w": "R16", "l": "R32", "q": "R64"}

PLACEHOLDER = re.compile(r"\{([A-Z0-9]+)\}")


def expand(template: str) -> list[str]:
    """Expand `{NAME}` placeholders over the Cartesian product of VOCAB."""
    # Resolve the `R{S}` shorthand first: the register class follows the size
    # suffix chosen for this expansion.
    out: list[str] = []
    suffix_dependent = "{R{S}}" in template or "%{R{S}}" in template
    sizes = VOCAB["S"] if ("{S}" in template) else [None]
    for size in sizes:
        t = template
        if size is not None:
            t = t.replace("{S}", size)
            if suffix_dependent:
                t = t.replace("{R{S}}", "{" + SUFFIX_REGS[size] + "}")
        names = PLACEHOLDER.findall(t)
        names = [n for n in dict.fromkeys(names) if n in VOCAB]
        if not names:
            out.append(t)
            continue
        for combo in itertools.product(*(VOCAB[n] for n in names)):
            s = t
            for n, v in zip(names, combo):
                s = s.replace("{" + n + "}", v)
            out.append(s)
    return out


# ─── Encoding ─────────────────────────────────────────────────────────────

@dataclass
class Encoding:
    ok: bool
    data: bytes | None
    error: str

    @property
    def hexs(self) -> str:
        if self.data is None:
            return f"<{self.error[:58]}>"
        return self.data.hex()


def run_objcopy(objcopy: str, obj: Path, out: Path) -> bytes:
    r = subprocess.run(
        [objcopy, "-O", "binary", "--only-section=.text", str(obj), str(out)],
        capture_output=True)
    if r.returncode != 0 or not out.exists():
        return b""
    return out.read_bytes()


def encode_lccc(lccc: str, src: Path, obj: Path, objcopy: str) -> Encoding:
    r = subprocess.run([lccc, "-c", str(src), "-o", str(obj)],
                       capture_output=True, text=True, timeout=120)
    if r.returncode != 0:
        msg = (r.stderr or r.stdout).strip().splitlines()
        return Encoding(False, None, msg[-1] if msg else "error")
    return Encoding(True, run_objcopy(objcopy, obj, obj.with_suffix(".bin")), "")


def encode_gas(gas: str, src: Path, obj: Path, objcopy: str) -> Encoding:
    r = subprocess.run([gas, "--64", "-o", str(obj), str(src)],
                       capture_output=True, text=True, timeout=120)
    if r.returncode != 0:
        msg = (r.stderr or r.stdout).strip().splitlines()
        return Encoding(False, None, msg[-1] if msg else "error")
    return Encoding(True, run_objcopy(objcopy, obj, obj.with_suffix(".bin")), "")



# A bare `jmp 1f` / `je 0b` cannot be assembled on its own: GAS rejects the
# numeric local label because the matching `1:` is not in the file.  That made
# every such case look like a FALSE-ACCEPT (LCCC assembles it, GAS errors out)
# when in fact neither assembler was being asked a meaningful question.
# Synthesise the labels the instruction refers to so the comparison tests the
# encoding instead of the harness.
_LOCAL_LABEL_RE = re.compile(r"(?:^|[\s,])(\d+)([fb])\b")


def local_label_scaffold(inst: str) -> tuple[str, str]:
    """Return (before, after) text defining any numeric local labels used."""
    before: list[str] = []
    after: list[str] = []
    for num, direction in _LOCAL_LABEL_RE.findall(inst):
        if direction == "b":
            before.append(f"{num}:")
        else:
            after.append(f"{num}:")
    return ("\n".join(before), "\n".join(after))



_SCALE1 = re.compile(r"\(,%([a-z0-9]+),1\)")
_ZERODISP = re.compile(r"(?<![0-9a-fx])0x0\(")


def _decode(objdump: str, data: bytes, tmp: Path) -> str:
    """Disassemble raw bytes and normalise away pure encoding choices."""
    if not data:
        return ""
    raw = tmp / "d.bin"
    raw.write_bytes(data)
    r = subprocess.run(
        [objdump, "-D", "-b", "binary", "-m", "i386:x86-64",
         "-M", "att", str(raw)],
        capture_output=True, text=True, timeout=120)
    out = []
    for line in r.stdout.splitlines():
        m = re.match(r"^\s+[0-9a-f]+:\s+((?:[0-9a-f]{2} )+)\s*\t(.*)$", line)
        if not m:
            continue
        insn = m.group(2).split("#")[0].strip()
        # A redundant scale-1 index and an explicit zero displacement are the
        # two spellings that differ only by encoding, not by meaning.
        insn = _SCALE1.sub(r"(%\1)", insn)
        insn = _ZERODISP.sub("(", insn)
        out.append(re.sub(r"\s+", " ", insn))
    return "\n".join(out)


def verify_shorter(objdump: str, l: Encoding, g: Encoding, tmp: Path) -> bool:
    """True when LCCC's shorter encoding decodes to the same instruction."""
    try:
        return _decode(objdump, l.data, tmp) == _decode(objdump, g.data, tmp)
    except (OSError, subprocess.SubprocessError):
        return False


# ─── Classification ───────────────────────────────────────────────────────

def classify(l: Encoding, g: Encoding) -> str:
    """Name the failure mode. The name determines how urgent it is."""
    if not g.ok and not l.ok:
        return "both-reject"
    if not g.ok and l.ok:
        # LCCC accepted something the reference rejects: it emitted bytes for
        # a malformed instruction, which is how a typo becomes a miscompile.
        return "FALSE-ACCEPT"
    if g.ok and not l.ok:
        return "REJECTS-VALID"
    if l.data == g.data:
        return "ok"
    if l.data is None or g.data is None:
        return "WRONG-BYTES"
    if len(l.data) == len(g.data):
        return "WRONG-BYTES"
    return "LONGER" if len(l.data) > len(g.data) else "SHORTER"


SEVERITY = {
    "FALSE-ACCEPT": 0,   # emits garbage for invalid input
    "WRONG-BYTES": 1,    # same length, different meaning
    "REJECTS-VALID": 2,  # cannot build valid code
    "LONGER": 3,         # correct but wastes I-cache
    "SHORTER": 4,        # smaller, but the decode check could not confirm it
    "BETTER": 7,         # smaller AND verified to decode identically
    "both-reject": 5,
    "ok": 6,
}


def main() -> int:
    ap = argparse.ArgumentParser(
        description="Per-instruction LCCC vs GNU as encoding differential.")
    ap.add_argument("--lccc", default=str(DEFAULT_LCCC))
    ap.add_argument("--as", dest="gas", default=os.environ.get("LCCC_GAS", "as"))
    ap.add_argument("--objdump",
                    default=os.environ.get("LCCC_OBJDUMP", "objdump"),
                    help="disassembler used to verify SHORTER encodings")
    ap.add_argument("--objcopy", default=os.environ.get("LCCC_OBJCOPY", "objcopy"))
    ap.add_argument("--file", type=Path, help="one instruction per line")
    ap.add_argument("--sweep", action="append", default=[],
                    help="template with {R64}/{IMM}/{S}... placeholders")
    ap.add_argument("--prologue", default=".text",
                    help="assembly emitted before each instruction")
    ap.add_argument("--only-diff", action="store_true")
    ap.add_argument("--quiet", action="store_true",
                    help="print only the summary (still exits non-zero on diff)")
    ap.add_argument("--max-report", type=int, default=200)
    ap.add_argument("--list-vocab", action="store_true")
    args = ap.parse_args()

    if args.list_vocab:
        for k, v in VOCAB.items():
            print(f"{{{k}}}: {' '.join(v)}")
        return 0

    for tool, label in ((args.lccc, "lccc"), (args.gas, "as"),
                        (args.objcopy, "objcopy")):
        if not (shutil.which(tool) or Path(tool).exists()):
            print(f"error: {label} not found at {tool!r}", file=sys.stderr)
            return 2

    insns: list[str] = []
    if args.file:
        insns += [l for l in args.file.read_text().splitlines()
                  if l.strip() and not l.strip().startswith("#")]
    for t in args.sweep:
        insns += expand(t)
    if not insns and not sys.stdin.isatty():
        insns += [l for l in sys.stdin.read().splitlines()
                  if l.strip() and not l.strip().startswith("#")]
    if not insns:
        print("error: no instructions given (use --file, --sweep or stdin)",
              file=sys.stderr)
        return 2

    results: list[tuple[str, str, Encoding, Encoding]] = []
    with tempfile.TemporaryDirectory(prefix="insndiff-") as td:
        tmp = Path(td)
        src = tmp / "i.s"
        for i, inst in enumerate(insns):
            before, after = local_label_scaffold(inst)
            parts = [args.prologue]
            if before:
                parts.append(before)
            parts.append(inst)
            if after:
                parts.append(after)
            src.write_text("\n".join(parts) + "\n")
            le = encode_lccc(args.lccc, src, tmp / f"l{i}.o", args.objcopy)
            ge = encode_gas(args.gas, src, tmp / f"g{i}.o", args.objcopy)
            kind = classify(le, ge)
            # A shorter encoding is only an improvement if it still means the
            # same thing. Confirm that here rather than leaving it to a human:
            # a shorter encoding that decodes differently is a miscompile.
            if kind == "SHORTER" and verify_shorter(args.objdump, le, ge, tmp):
                kind = "BETTER"
            results.append((inst, kind, le, ge))

    results.sort(key=lambda r: (SEVERITY.get(r[1], 9), r[0]))
    shown = 0
    for inst, kind, le, ge in results:
        if args.quiet:
            break
        if args.only_diff and kind in ("ok", "both-reject", "BETTER"):
            continue
        if shown >= args.max_report:
            print(f"... ({len(results) - shown} more)")
            break
        tag = "ok  " if kind == "ok" else kind
        print(f"{tag:<14} {inst:<44} lccc={le.hexs:<30} gas={ge.hexs}")
        shown += 1

    counts: dict[str, int] = {}
    for _, kind, _, _ in results:
        counts[kind] = counts.get(kind, 0) + 1
    bad = sum(v for k, v in counts.items() if k not in ("ok", "both-reject"))
    summary = "  ".join(f"{k}={v}" for k, v in
                        sorted(counts.items(), key=lambda kv: SEVERITY.get(kv[0], 9)))
    print(f"\n=== insndiff: {len(results)} instruction(s): {summary} ===")
    print(f"oracle: {args.gas}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
