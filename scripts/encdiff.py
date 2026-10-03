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
  * BEATS      LCCC is shorter than every oracle and its bytes round-trip to
               the same instruction as every shortest oracle encoding.
  * WRONG      LCCC's bytes decode to a different instruction, or it rejects
               input every oracle accepts.
  * UNVERIFIED-BEATS / UNVERIFIED-BYTES
               the disassembler could not establish semantic equivalence;
               these are never reported as a win or a pass.

The round-trip check is always on. A shorter encoding that decodes to
something else is a miscompile, not an optimisation: LCCC's bytes are
compared with every shortest oracle encoding, and `BEATS` is emitted only when
all comparisons succeed. Missing/unsupported disassembly fails closed as an
`UNVERIFIED-*` verdict rather than being counted as a win.

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

    # i686: local lccc-i686 and GAS --32, remote compilers with -m32
    scripts/encdiff.py --32 --lccc target/fastbuild/lccc-i686 \
        --casefiles tests/asm-diff/i686/*.casefile
"""
from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import json
import os
import random
import re
import subprocess
import sys
import time
import tempfile
import urllib.error
import urllib.request
from dataclasses import dataclass, field
from pathlib import Path

API = "https://godbolt.org/api"

# The cache used to default to the RELATIVE path ".godbolt-cache", so the
# directory this tool wrote to depended on the working directory it was
# invoked from: run it from anywhere but the repository root and it silently
# built a second, private cache in that directory, re-issuing every request
# and scattering .godbolt-cache/ over the filesystem. It now uses the one
# absolute, tree-wide location shared with scripts/godbolt.py,
# scripts/codegen_oracle.py and tools/oracle/godbolt_oracle.py.
sys.path.insert(0, str(Path(__file__).resolve().parent))
import godbolt_cache  # noqa: E402
import godbolt  # noqa: E402  -- for compiler_fingerprint (drift-safe cache keys)
CACHE = godbolt_cache.CACHE
_OBJDUMP = os.environ.get("LCCC_OBJDUMP", "objdump")
TIMEOUT = int(os.environ.get("GODBOLT_TIMEOUT", "120"))

# Remote oracles. Every one of these has its own integrated assembler, so they
# are independent implementations of the same encoding rules -- exactly what a
# differential needs. Pinned ids keep results reproducible; `--compiler` can
# override for a one-off check against a newer build.
REMOTE_ORACLES = {
    "clang": "cclang2310",
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
                 prologue: str = ".text", bits32: bool = False) -> Encoding:
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
        cmd = [tool] + (["-m32"] if bits32 else []) + [
            "-c", str(src), "-o", str(obj)]
    else:
        cmd = [tool, "--32" if bits32 else "--64",
               "-o", str(obj), str(src)]
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

def _post(cid: str, source: str, args: str) -> dict:
    """Compile `source` remotely, returning the API's JSON response."""
    # Through the cache shared tree-wide, under its own namespace: this stores
    # the raw API reply (including `opcodes`), which is a different shape from
    # the assembly held in `att-v2` and the execution records in `oracle-v1`.
    # A corrupt record is treated as a miss and overwritten, never fatal.
    fp = godbolt.compiler_fingerprint(cid)
    hit = godbolt_cache.load_json(godbolt_cache.NS_ENCDIFF, cid, fp, args, source)
    if hit is not None:
        return hit

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
    godbolt_cache.store_json(godbolt_cache.NS_ENCDIFF, out, cid, fp, args, source)
    return out


_TRANSIENT = (urllib.error.URLError, TimeoutError, json.JSONDecodeError, OSError)


def _post_retry(cid: str, source: str, args: str, *, attempts: int = 5):
    """POST with retries for transient failures (429/5xx/timeouts/resets).

    One failed batch used to poison 60 instructions; Godbolt rate-limits
    burst traffic, so back off exponentially with jitter. Non-retryable
    4xx (other than 429) fail fast: they are deterministic.
    """
    last: Exception | None = None
    for attempt in range(attempts):
        try:
            return _post(cid, source, args)
        except urllib.error.HTTPError as e:
            last = e
            if e.code != 429 and e.code < 500:
                raise
        except _TRANSIENT as e:
            last = e
        if attempt + 1 < attempts:
            time.sleep(0.5 * (2 ** attempt) + random.uniform(0, 0.5))
    assert last is not None
    raise last


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
        r = _post_retry(cid, source, args)
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
        r = _post_retry(cid, _batch_source(group), args)
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
    # The row's byte-exact pin, from a `# byte-exact <hex>' annotation
    # (see split_byte_pin): the strongest row contract, checked BEFORE
    # any law, canonicaliser or round-trip. None on unpinned rows.
    byte_pin: bytes | None = None


# A corpus row may pin its LCCC bytes byte-exactly:
#     mov %ds:4(,%ebp,1), %rax   # byte-exact 3e67488b4504
# The annotation is stripped from the instruction before assembly and
# enforced in classify() ahead of every other rule, so no downstream
# machinery — not the dead-segment strip, not a same-rendering oracle,
# not any future unification law — can launder a pinned byte difference
# into a verdict. It exists for rows whose bytes ARE the claim but whose
# verdict-level comparison cannot see them: the flip rows (the folded
# view legitimately differs from GAS's raw bytes by the dead prefix, so
# the gate's canonicaliser must unify exactly there — the prefix byte's
# truth would otherwise live only in the Rust unit tests) and the APX
# folds (whose correctness argument is byte-identity with the base-form
# spelling GAS itself emits).
_BYTE_PIN = re.compile(r"#\s*byte-exact\s+((?:[0-9a-f]{2})+)\s*$",
                     re.IGNORECASE)


def split_byte_pin(text: str) -> tuple[str, bytes | None]:
    """Split one instruction line into (instruction, pinned LCCC bytes).

    A line without the annotation returns (text, None) unchanged. A line
    carrying a MALFORMED annotation (anything byte-exact-shaped that does
    not match the strict `hex bytes to end of line' grammar) is an error,
    not a silently-ignored comment: a typo'd pin must never quietly weaken
    the row's contract back to verdict-only. The tag and the hex digits
    are matched case-insensitively — `# BYTE-EXACT <hex>' is the same pin
    as `# byte-exact <hex>' — so no near-miss spelling of the TAG can
    fall through as prose either.
    """
    m = _BYTE_PIN.search(text)
    if m:
        return text[:m.start()].rstrip(), bytes.fromhex(m.group(1))
    # Liberal in what it flags: any comment that LOOKS like a pin
    # annotation in any near-miss spelling (byte exact / byte_exact /
    # ByteExact / byte-exact with junk hex) is an error, not prose — a
    # typo'd pin must never silently weaken the row back to verdict-only.
    if re.search(r"#\s*byte[\s_-]*exact\b", text, re.IGNORECASE):
        raise ValueError(
            f"malformed byte-exact annotation: {text.strip()!r} "
            "(expected `# byte-exact <hex bytes>', two hex digits per byte)")
    return text, None


def harvest_byte_pins(insns: list[str]) -> tuple[list[str], dict[str, bytes]]:
    """Strip pin annotations from every line and enforce the pin policy.

    Returns (stripped lines, pins keyed by stripped text). Raises
    ValueError on malformed annotations, on the same instruction text
    pinned twice with CONFLICTING bytes, and on the same text appearing
    both pinned and unpinned: the deduplicated row carries one contract,
    and it must be unambiguous — never decided by corpus ordering or a
    dict's last-write-wins.
    """
    byte_pins: dict[str, bytes] = {}
    occurrence: dict[str, str] = {}   # stripped text -> "pinned" | "unpinned"
    stripped: list[str] = []
    for one in insns:
        text, pin = split_byte_pin(one)
        kind = "pinned" if pin is not None else "unpinned"
        prior_kind = occurrence.get(text)
        if prior_kind is not None and prior_kind != kind:
            raise ValueError(
                f"byte-exact pin policy: {text!r} appears both "
                f"{prior_kind} and {kind}; pin every occurrence or none "
                "— the deduplicated row's contract must be unambiguous")
        occurrence[text] = kind
        if pin is not None:
            prior = byte_pins.get(text)
            if prior is not None and prior != pin:
                raise ValueError(
                    f"byte-exact pin policy: {text!r} is pinned with "
                    f"conflicting bytes: {prior.hex()} then {pin.hex()} "
                    "— last-write-wins is not a contract")
            byte_pins[text] = pin
        stripped.append(text)
    return stripped, byte_pins


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
    # Integer 0F-map ops LCCC may source-swap to reach VEX2 (not FP add/mul).
    "vpmuludq", "vpsadbw", "vpmaddwd",
}
# VEX packed-FP compare pseudos with operand-symmetric predicates
# ((imm&31)&7 in {eq,unord,neq,ord}, every _q/_s flavor + true/false):
# objdump always disassembles VEX C2 to these spellings (never `vcmpps $imm`),
# and LCCC/clang/icx may exchange the sources to reach the 2-byte VEX prefix.
# Scalar (ss/sd) and ordered-predicate pseudos are deliberately absent: the
# merge lane makes scalar swaps unsound and no oracle swaps ordered compares.
_COMMUTATIVE_VEX_CMP = {
    "vcmpeqps", "vcmpeqpd", "vcmpunordps", "vcmpunordpd",
    "vcmpneqps", "vcmpneqpd", "vcmpordps", "vcmpordpd",
    "vcmpeq_uqps", "vcmpeq_uqpd", "vcmpfalseps", "vcmpfalsepd",
    "vcmpneq_oqps", "vcmpneq_oqpd", "vcmptrueps", "vcmptruepd",
    "vcmpeq_osps", "vcmpeq_ospd", "vcmpunord_sps", "vcmpunord_spd",
    "vcmpneq_usps", "vcmpneq_uspd", "vcmpord_sps", "vcmpord_spd",
    "vcmpeq_usps", "vcmpeq_uspd", "vcmpfalse_osps", "vcmpfalse_ospd",
    "vcmpneq_osps", "vcmpneq_ospd", "vcmptrue_usps", "vcmptrue_uspd",
}
_VEX3 = re.compile(r"^(v\S+)\s+(%\S+),(%\S+),(%\S+)$")
_VEXCMP_IMM = re.compile(r"^(vcmpps|vcmppd)\s+\$(0x[0-9a-f]+|\d+),(%\S+),(%\S+),(%\S+)$")


def _vcmp_imm_symmetric(text: str) -> bool:
    """True when a VCMP numbered imm selects an operand-symmetric predicate.

    objdump prints the pseudo-mnemonic only for imm 0-31; larger immediates
    (GAS accepts the full imm8, e.g. `$0xab`) disassemble numbered. The
    predicate lives in the low 5 bits; relations eq/unord/neq/ord (imm&7 in
    {0,3,4,7}) are symmetric in every flavor, true/false included.
    """
    try:
        imm = int(text, 0)
    except ValueError:
        return False
    return ((imm & 31) & 7) in (0, 3, 4, 7)


_GP32_TO_64 = {
    "eax": "rax", "ebx": "rbx", "ecx": "rcx", "edx": "rdx",
    "esi": "rsi", "edi": "rdi", "ebp": "rbp", "esp": "rsp",
    **{f"r{n}d": f"r{n}" for n in range(8, 32)},
}
# 64-bit GPR names including the APX EGPR range: `movabs $val,%rN` (the
# imm64 form) canonicalises onto the same `mov $val,%rN` spelling as the
# zero-extending imm32 form when the value fits unsigned 32 bits — they
# write identical register contents (x86-64 zero-extends every 32-bit
# write), so a length comparison between them is a pure encoding choice.
_GP64_FULL = {
    "rax", "rbx", "rcx", "rdx", "rsi", "rdi", "rbp", "rsp",
    *(f"r{n}" for n in range(8, 32)),
}
_MOV_IMM = re.compile(r"^(movabs|mov)\s+\$(0x[0-9a-f]+|\d+),%(\w+)$")
# TEST and XCHG: the two GPR instructions whose operand pair is
# symmetric — TEST reads two sources into EFLAGS (AND is
# order-independent), XCHG writes each operand with the other's value
# (the exchange is its own inverse). Remote oracles split on the modrm
# orientation: ICC's -O0 encoder emits the commuted TEST form (`40 84 c5`
# for `test %bpl,%al`) and clang/ICC/ICX the commuted XCHG form (`40 86 c5`
# for `xchgb %bpl,%al`) where GAS/GCC/lccc emit `40 84 e8` / `40 86 e8`.
# Sorting the two operands unifies the spellings, like the commutative VEX
# forms. No other GPR instruction qualifies: CMP/UCOMISx operand order is
# observable, and every other two-operand ALU op writes a destination.
# REGISTER pair only: with a memory operand both source orders assemble
# to the SAME bytes (one encoding shape), so no unification is needed
# -- and a ([^,]+),(.+) split tore SIB operands apart at their commas
# (`0x10(%rbx` + `%rcx,4),%eax`).
_TEST = re.compile(r"^(test|xchg)\s+(%[a-z0-9]+),(%[a-z0-9]+)$")
# Selector moves and selector stores, 64-bit mode only. The SDM lists
# `REX.W + 8C /r MOV r64/m16, Sreg`, `REX.W + 8E /r MOV Sreg, r/m64` and
# the REX.W SLDT/STR r64 rows separately from the 32-bit rows, but every
# 32-bit GPR write in 64-bit mode zero-fills the upper half, so the
# 32-bit-view form leaves the destination holding exactly the
# zero-extended 16-bit selector the 64-bit-view form stores — identical
# architectural state, either size class. GAS/GCC/ICC/lccc take the no-W
# row (on the legacy path it is also one byte SHORTER: `8c e0` vs
# `48 8c e0`); clang/icx take the W row. objdump renders the size view
# (`mov %fs,%eax` vs `mov %fs,%rax`), which is why the round-trip needs
# this rule. Only the 32-bit view unifies with the 64-bit view: a
# 16-bit write preserves the upper bits and must stay distinct. The 8E
# register SOURCE is consumed at bits 15:0 only, so its size view is
# neutral there too. In 32-bit mode nothing unifies (a 16-bit selector
# write preserves bits 31:16 — a real difference).
_SELREG = "%(?:es|cs|ss|ds|fs|gs)"
_SEL_FROM = re.compile(rf"^mov\s+({_SELREG}),%(\w+)$")
_SEL_TO = re.compile(rf"^mov\s+%(\w+),({_SELREG})$")
_SEL_STORE = re.compile(r"^(sldt|str)\s+%(\w+)$")
# Direct branch whose objdump spelling differs by encoding choice.
# objdump renders a 66-prefixed SHORT branch (`66 eb d8`, `66 74 d8`)
# as `data16 jmp/je` — on hardware the 66 is a dead prefix there
# (64-bit: near branches ignore operand-size prefixes per the Intel
# SDM; 32-bit: the only effect is EIP truncation to 16 bits, which no
# in-corpus payload can observe), so the spelling is unified with the
# plain short form in BOTH modes. The LOOP family rides the same rule in
# both modes, for different reasons: on 64-bit hardware `66 e2` still
# decrements the full RCX (verified; the prefix is dead); in 32-bit mode
# `66 e2` still counts ECX and only truncates IP to 16 bits on the taken
# branch — the counter WIDTH (CX vs ECX) is selected by the
# ADDRESS-SIZE prefix (0x67; objdump renders `67 e2` as `loopw`), never
# by 0x66 — so for any in-payload target the transfer is the
# same (the jmpw argument). In 32-bit mode `jmpw` (66 e9 rel16) is the same
# transfer as `jmp` (e9 rel32) for any in-payload target (< 64 KiB)
# and is unified. `callw` is NEVER unified with `call`: the 16-bit call
# pushes a 2-byte return address where the 32-bit call pushes 4 — a
# real semantic difference in every mode. In 64-bit mode `jmpw` and
# `callw` render the truncated rel16 rows the DECODER REJECTS (it reads
# a full 4-byte displacement; see invalid_64_data16_branch) — they are
# never equivalence partners.
_DEAD66_BRANCH = re.compile(r"^data16\s+(j[a-z]+|loop[a-z]*)\b")
_BRANCH_W32 = re.compile(r"^jmpw\b")
# 64-bit mode ignores the ES/DS/SS segment overrides on data accesses
# (Intel SDM Vol. 2, effect of segment-override prefixes in 64-bit mode:
# only FS and GS are honored; CS/DS/ES/SS are fixed at base 0). objdump
# renders the dead byte as a leading `ds `/`es `/`ss ` mnemonic token, so
# encodings differing only by it decode to the same instruction. The
# spellings that must NOT unify are spelled differently by objdump and
# therefore cannot match this regex: FS/GS render inline
# (`mov %fs:0x4(%rax),%rax`), NOTRACK renders as its own mnemonic
# (`notrack jmp *%rax`, byte 3e on an indirect branch — semantically real
# under CET), and branch hints render as `,pt`/`,pn` suffixes (`je,pt`).
# CS (0x2e) is left alone too: on indirect branches it is the
# historically-defined hint partner, and there is no reason to unify a
# byte any oracle chose to emit. 32-bit mode strips nothing: every
# override selects a real descriptor there.
#
# The law is NOT applied unconditionally: a
# corpus whose only dead-segment divergence is the index-fold family was
# no reason to blind every other 64-bit comparison — the token would be
# invisible to ANY corpus, ad-hoc --insn run or future casefile, and a
# wrongly-dropped/kept byte there would pass silently. Instead the strip
# is scoped by _ROW_SEG_FOLD (below) to exactly the rows where lccc's
# FOLDED view of the operand can disagree with GAS's raw view: the
# index-only scale-1 fold moving rbp/ebp into the base slot flips the
# default-segment class (DS -> SS; the elision match in the encoder is
# name-based rbp/ebp/rsp/esp — rsp/esp cannot be an index, and r12/r13
# fold to non-matching names, so the flip set is exactly {rbp, ebp}).
# Everywhere else both encoders decide identically, the strip would be a
# no-op for CORRECT bytes — and the only thing it could ever hide is a
# regression, so it is not applied. The prefix byte on the two flip rows
# is byte-pinned in Rust instead (encoder mod.rs
# index_fold_tests::fold_decides_segment_elision_on_the_folded_view).
_SEG_DEAD64 = re.compile(r"^(?:ds|es|ss)\s+(?=[a-z])")
# A row whose SOURCE asks for a segment override on the index-only
# scale-1 operand that folds rbp/ebp into the base slot: the one family
# where the encoder's folded-view segment decision can diverge from
# GAS's raw-view bytes (see the _SEG_DEAD64 law comment). Displacements:
# integer or none — symbol displacements never fold (the encoder guard),
# so those rows keep full discrimination too.
_ROW_SEG_FOLD = re.compile(
    r"%(?:cs|ds|es|fs|gs|ss)\s*:\s*"
    r"(?:[-+]?(?:0[xX][0-9a-fA-F]+|\d+))?"
    r"\(\s*,\s*%(?:r|e)bp\s*,\s*1\s*\)")
# Direct branch to an absolute target: rewritten with BOTH comparison
# invariants (see _branch_marker).
_BRANCH_TARGET = re.compile(
    r"^((?:data16\s+)?(?:j[a-z]+|jmp|call|loop[a-z]*))\s+(0x[0-9a-f]+)$")
# A rewritten branch marker: mnemonic, absolute target, end-relative
# displacement. Two encodings of one branch are equivalent when EITHER
# the absolute targets match (label BEFORE the instruction: its position
# is independent of this instruction's encoding) OR the end-relative
# displacements match (label AFTER: it rides at a fixed distance from
# the instruction end). Comparing only one arm misjudges the other
# direction — end-relative alone breaks backward labels, absolute alone
# breaks forward ones.
_BRANCH_MARK = re.compile(r"^(.*?) @a=(0x[0-9a-f]+) @d=([+-]0x[0-9a-f]+)$")


def _movq_request_value(insn: str) -> int | None:
    """The requested immediate of `movq $V, %reg` (row source text)."""
    m = re.match(r"^movq\s+\$(0x[0-9a-f]+|-?\d+),\s*%\w+$", insn.strip())
    if not m:
        return None
    try:
        return int(m.group(1), 0)
    except ValueError:
        return None


def _sext_imm32_form(data: bytes) -> int | None:
    """The sign-extended value a W imm32 MOV form writes, or None.

    The imm32-sign-extending MOV r64 shapes are C7 /0 mod=11 under
    REX.W (7 bytes) and under REX2 with the W bit (8 bytes, `d5 18 c7
    /0 imm32` -- both lccc and GAS emit that form for plain values, so
    an oracle using it for a bit31-set request stores V - 2**32: the
    ICC miscompile class in its REX2 variant). The B8+rd opcode has NO
    imm32 form under W: REX.W/REX2.W B8 is movabs and always consumes
    a FULL imm64, so a 6-byte `48 b8 imm32` buffer is a truncated,
    undecodable instruction and must return None (the old arm matched
    exactly that phantom). The no-W 32-bit writes (b8 imm32, 41 b8
    imm32, d5 10 b8 imm32) zero-extend -- not sign-extending forms --
    and memory forms return None. There is deliberately no EVEX arm:
    plain MOV is not EVEX-promotable (`{evex} movq` is rejected), so
    no 62-prefixed C7 /0 row can arise.
    """
    if len(data) == 7 and data[1] == 0xC7:
        rex, modrm, imm_at = data[0], data[2], 3
        w = 0x48 <= rex <= 0x4F and (rex & 0x08) != 0
    elif len(data) == 8 and data[0] == 0xD5 and data[2] == 0xC7:
        rex2, modrm, imm_at = data[1], data[3], 4
        w = (rex2 & 0x08) != 0
    else:
        return None
    if not w:
        return None
    # C7 /0 is MOV: mod must be register-direct and the /reg field 000
    # (C7 /1-/7 encode no instruction; a stray one is not a MOV).
    if modrm & 0b11_111_000 != 0b11_000_000:
        return None
    imm32 = int.from_bytes(data[imm_at:imm_at + 4], "little")
    return imm32 - (1 << 32) if imm32 >= (1 << 31) else imm32


def invalid_64_sext_imm32(insn: str, data: bytes) -> bool:
    """True when `data` is a REX.W imm32 form that miscompiles `insn`.

    For `movq $V, %reg` with V in [2**31, 2**32) (a positive u32 with
    bit31 set), the REX.W C7 / B8 encodings store sext(imm32): the imm32
    bits equal V, but the stored 64-bit value is V - 2**32, not V. ICC's
    -O0 encoder emits this shape for exactly these requests --
    objdump-verified (`mov $0xffffffff80000000,%r15` for the requested
    0x80000000). Such an oracle form is not the requested program and is
    partitioned out of the comparison, like the truncated data16 rows.
    """
    v = _movq_request_value(insn)
    if v is None or not (1 << 31) <= v < (1 << 32):
        return False
    stored = _sext_imm32_form(data)
    return stored is not None and stored != v and (stored & 0xFFFFFFFF) == v


def invalid_64_data16_branch(data: bytes) -> bool:
    """True for a 66-prefixed near branch with a 2-byte displacement
    field — architecturally invalid in 64-bit mode.

    Intel SDM: operand-size prefixes have no effect on near branches in
    64-bit mode; the decoder consumes a FULL 4-byte displacement after
    `66 e8` / `66 e9` / `66 0f 8x` regardless of the prefix. A payload
    that stops after a 2-byte field therefore desynchronises the
    instruction stream — the decoder reads two bytes of whatever
    follows and jumps through them (hardware-verified on a Xeon: the
    fault RIP is exactly insn_end + disp32(read from past the payload)).
    GAS 2.47 emits these truncated rows for `data16 jmp/je/call` with
    far or external targets (with R_X86_64_PC16); the payload lengths
    4 (e8/e9) and 5 (0f 8x) identify them unambiguously — the valid
    dead-prefix forms are 6 and 7 bytes.
    """
    if len(data) >= 3 and data[0] == 0x66:
        if data[1] in (0xE8, 0xE9):
            return len(data) == 4
        if data[1] == 0x0F and 0x80 <= data[2] <= 0x8F:
            return len(data) == 5
    return False


def _branch_marker(insn: str, addr: int, nbytes: int) -> str:
    """Rewrite a direct branch's absolute target with both invariants."""
    m = _BRANCH_TARGET.match(insn)
    if not m:
        return insn
    target = int(m.group(2), 16)
    return f"{m.group(1)} @a={target:#x} @d={target - (addr + nbytes):+#x}"


def _stream_equal(a: str, b: str) -> bool:
    """Compare two canonical instruction streams.

    Plain lines must match exactly. Branch-marker lines match when the
    mnemonic agrees and EITHER branch invariant does (see
    _branch_marker) — the encoding-independent statement of "transfers
    to the same place" for a corpus whose labels sit immediately before
    or after the instruction.
    """
    la, lb = a.split("\n"), b.split("\n")
    if len(la) != len(lb):
        return False
    for x, y in zip(la, lb):
        if x == y:
            continue
        ma, mb = _BRANCH_MARK.match(x), _BRANCH_MARK.match(y)
        if ma and mb and ma.group(1) == mb.group(1) and (
                ma.group(2) == mb.group(2) or ma.group(3) == mb.group(3)):
            continue
        return False
    return True


def _canon_insn(insn: str, bits32: bool = False, seg_dead64: bool = False) -> str:
    """Canonicalise disassembly spellings that differ only by encoding choice.

    `seg_dead64` opts the comparison into the dead-ES/DS/SS unification
    (_SEG_DEAD64): it is set per ROW (see classify) for exactly the
    index-fold rows whose folded view can legitimately differ from GAS's
    bytes by a dead segment byte. The default keeps every other 64-bit
    comparison fully discriminating on the prefix byte.
    """
    insn = insn.split("#")[0].strip().lower()
    # Objdump marks an EVEX-only mnemonic's legal VEX row with a GNU pseudo
    # prefix (`{vex} vpdpbusds`). It describes the selected encoding, not a
    # different architectural instruction, so remove it for semantic compare.
    insn = re.sub(r"^\{vex(?:2|3)?\}\s+", "", insn)
    # 66-prefixed SHORT branches (`data16 jmp 0x3` = `66 eb 01`) are the
    # dead-prefix spellings of the plain short rows in every mode; the
    # LOOP family unifies in both modes too (64-bit: hardware-dead;
    # 32-bit: EIP-truncation only, counter width is address-size
    # selected -- see the block comment above). In 32-bit mode `jmpw`
    # additionally transfers to the same place as `jmp` for any
    # in-payload target.
    insn = _DEAD66_BRANCH.sub(r"\1", insn)
    if bits32:
        insn = _BRANCH_W32.sub("jmp", insn)
    elif seg_dead64:
        # ES/DS/SS overrides are architecturally dead on 64-bit data
        # accesses; on the index-fold rows (seg_dead64) encodings differing
        # only by the byte must compare equal — lccc's folded form drops
        # the no-op override where GAS's SIB form keeps the meaningful one
        # (see the _SEG_DEAD64 law comment).
        insn = _SEG_DEAD64.sub("", insn)
    insn = _SCALE1.sub(r"(%\1)", insn)
    insn = _ZERODISP.sub("(", insn)
    insn = re.sub(r"\s+", " ", insn)
    m = _VEX3.match(insn)
    if m and (m.group(1) in _COMMUTATIVE_VEX or m.group(1) in _COMMUTATIVE_VEX_CMP):
        a, b = sorted((m.group(2), m.group(3)))
        insn = f"{m.group(1)} {a},{b},{m.group(4)}"
    m = _VEXCMP_IMM.match(insn)
    if m and _vcmp_imm_symmetric(m.group(2)):
        a, b = sorted((m.group(3), m.group(4)))
        insn = f"{m.group(1)} ${m.group(2)},{a},{b},{m.group(5)}"
    m = _MOV_IMM.match(insn)
    if m:
        val = int(m.group(2), 0)
        reg = m.group(3)
        if 0 <= val <= 0xFFFFFFFF:
            # Zero-extension equivalence: writing a 32-bit register (the
            # imm32 opcode, incl. the REX2/EGPR rows objdump prints as
            # %r16d) and movabs-ing the same zero-extended value into the
            # 64-bit register produce identical register contents, so both
            # canonicalise to `mov $val,%r64` (covers %r16d-%r31d too).
            if reg in _GP32_TO_64:
                insn = f"mov ${val:#x},%{_GP32_TO_64[reg]}"
            elif reg in _GP64_FULL:
                insn = f"mov ${val:#x},%{reg}"
    # TEST and XCHG commute their operand pair (ICC emits the commuted
    # TEST form, clang/ICC/ICX the commuted XCHG form). Sorting makes both
    # spellings compare equal; no other GPR instruction gets this rule
    # (CMP/UCOMISx operand order is observable, and every two-operand ALU
    # op other than XCHG writes a destination).
    m = _TEST.match(insn)
    if m:
        a, b = sorted((m.group(2), m.group(3)))
        insn = f"{m.group(1)} {a},{b}"
    if not bits32:
        # Selector moves/stores: unify the 32-bit register view with the
        # 64-bit view (the block comment at the regex definitions proves
        # the equivalence; 16-bit views stay distinct, and in 32-bit mode
        # nothing unifies at all).
        m = _SEL_FROM.match(insn)
        if m and m.group(2) in _GP32_TO_64:
            insn = f"mov {m.group(1)},%{_GP32_TO_64[m.group(2)]}"
        m = _SEL_TO.match(insn)
        if m and m.group(1) in _GP32_TO_64:
            insn = f"mov %{_GP32_TO_64[m.group(1)]},{m.group(2)}"
        m = _SEL_STORE.match(insn)
        if m and m.group(2) in _GP32_TO_64:
            insn = f"{m.group(1)} %{_GP32_TO_64[m.group(2)]}"
    return insn


def decodes_same(objdump: str, a: bytes, b: bytes,
                 bits32: bool = False, seg_dead64: bool = False) -> bool | None:
    """Compare disassembly, returning None when it cannot be verified.

    False means both byte strings decoded successfully but to different
    instruction text. None is deliberately distinct: a missing/old objdump,
    a failed invocation, empty bytes, or undecodable data must not be treated
    as proof that a shorter encoding is correct (or as proof that it is wrong).
    """
    def dis(data: bytes) -> str | None:
        if not data:
            return None
        with tempfile.TemporaryDirectory(prefix="encdiff-dis-") as td:
            raw = Path(td) / "d.bin"
            raw.write_bytes(data)
            r = subprocess.run(
                [objdump, "-D", "-b", "binary", "-m",
                 "i386" if bits32 else "i386:x86-64",
                 "-M", "att", str(raw)],
                capture_output=True, text=True, timeout=120)
        if r.returncode != 0:
            return None
        out = []
        for line in r.stdout.splitlines():
            m = re.match(
                r"^\s+([0-9a-f]+):\s+((?:[0-9a-f]{2} )+)\s*\t(.*)$", line)
            if not m:
                continue
            insn = _canon_insn(m.group(3), bits32=bits32, seg_dead64=seg_dead64)
            # Branch targets: rewrite with both comparison invariants so
            # a short and a near encoding of the same branch compare
            # equal exactly when they transfer to the same place (see
            # _branch_marker / _stream_equal).
            insn = _branch_marker(
                insn, int(m.group(1), 16), len(m.group(2).split()))
            # Objdump renders undecodable bytes as `.byte` (and some versions
            # use `(bad)`). Two undecodable streams are not equivalent code.
            if not insn or insn == "(bad)" or insn.startswith(".byte"):
                return None
            out.append(insn)
        return "\n".join(out) if out else None

    try:
        da, db = dis(a), dis(b)
    except (OSError, subprocess.SubprocessError):
        return None
    if da is None or db is None:
        return None
    return _stream_equal(da, db)


def _roundtrip_same_as(row: Row, references: list[bytes],
                       bits32: bool = False, seg_dead64: bool = False) -> bool | None:
    """Require the candidate to decode like every distinct reference form.

    `None` is fail-closed: without a usable disassembly there is no semantic
    evidence for a size win (or for a pass when the bytes differ).
    """
    if not row.lccc.ok or row.lccc.data is None or not references:
        return None
    results = [decodes_same(_OBJDUMP, row.lccc.data, ref, bits32=bits32,
                            seg_dead64=seg_dead64)
               for ref in sorted(set(references))]
    if any(result is False for result in results):
        return False
    if results and all(result is True for result in results):
        return True
    return None


def _classify_roundtrip(row: Row, references: list[bytes], success: str,
                        bits32: bool = False, seg_dead64: bool = False) -> None:
    """Assign a byte-different verdict only after semantic round-trip proof."""
    same = _roundtrip_same_as(row, references, bits32=bits32,
                              seg_dead64=seg_dead64)
    if same is True:
        row.verdict = success
        row.note = (row.note + " | " if row.note else "") + \
                   "round-trip verified against shortest oracle encoding(s)"
    elif same is False:
        row.verdict = "WRONG-BYTES"
        row.note = (row.note + " | " if row.note else "") + \
                   "LCCC disassembly differs from a shortest oracle encoding"
    else:
        row.verdict = "UNVERIFIED-BEATS" if success == "BEATS" else "UNVERIFIED-BYTES"
        row.note = (row.note + " | " if row.note else "") + \
                   "objdump could not verify semantic equivalence"


def classify(row: Row, bits32: bool = False) -> None:
    # A byte-exact pin is the STRONGEST row contract, so it is checked
    # FIRST — before the dead-segment law, before any partitioning or
    # round-trip, before a verdict of any kind: nothing downstream (not
    # the _ROW_SEG_FOLD strip, not a same-rendering oracle) can launder
    # a pinned byte difference into a pass. This is what makes the flip
    # rows' segment byte a CI record: the dead-segment canon deliberately
    # tolerates any ES/DS/SS choice on opted-in rows (both views are the
    # same 64-bit flat-mode program), so the prefix byte's truth on those
    # rows cannot be a verdict — it is the pin (the byte-level fact
    # previously lived only in Rust unit tests, and a dropped 0x3e on
    # the ds flip row classifies BEATS straight through the strip —
    # exactly the laundering the pin exists to catch).
    if row.byte_pin is not None:
        got = row.lccc.data if (row.lccc.ok and row.lccc.data is not None) else None
        if got != row.byte_pin:
            row.verdict = "WRONG-BYTES"
            row.note = ("byte-exact pin violated: pinned "
                        + row.byte_pin.hex() + ", lccc "
                        + (got.hex() if got is not None
                           else "did not assemble the row"))
            return
    # The dead-ES/DS/SS unification is row-scoped (see _SEG_DEAD64): only
    # the segment+index-fold rows — the family whose folded view can
    # legitimately differ from GAS's raw bytes by one dead byte — opt in;
    # every other row keeps full prefix discrimination. The opt-in is
    # decided on the COMMENT-STRIPPED source, exactly like _canon_insn:
    # corpus rows may carry trailing `#' comments, and a comment quoting a
    # flip-shaped operand (`... # unlike %ds:4(,%rbp,1), this row ...')
    # must not opt its row into the strip — that would launder a
    # dead-prefix regression on a non-flip row into a pass. This was
    # proven with the shipped predicate before the fix.
    seg_fold = _ROW_SEG_FOLD.search(row.insn.split("#")[0]) is not None
    ok_oracles = {k: v for k, v in row.oracles.items() if v.ok and v.data is not None}

    if not ok_oracles:
        row.verdict = "NO-ORACLE" if row.lccc.ok else "both-reject"
        return

    # In 64-bit mode, a 66-prefixed near branch whose displacement field
    # is 2 bytes is architecturally invalid (the decoder reads a full
    # 4-byte field; see invalid_64_data16_branch). Such oracle bytes are
    # not a length race to win or lose — they are not encodings at all —
    # so they are partitioned out before any comparison. The same bytes
    # from LCCC would be a codegen bug and are reported immediately.
    if not bits32:
        if (row.lccc.ok and row.lccc.data is not None
                and invalid_64_data16_branch(row.lccc.data)):
            row.verdict = "WRONG-BYTES"
            row.note = ("LCCC emitted a 66-prefixed near branch with a"
                        " 2-byte displacement field — architecturally"
                        " invalid in 64-bit mode (the decoder consumes a"
                        " full 4-byte field and desynchronises)")
            return
        valid = {k: v for k, v in ok_oracles.items()
                 if not invalid_64_data16_branch(v.data)}
        invalid_who = sorted(set(ok_oracles) - set(valid))
        if not valid:
            row.verdict = "ORACLE-INVALID"
            row.note = ("every oracle form is a truncated 66-prefixed"
                        " near branch (2-byte displacement field):"
                        " hardware reads a full 4-byte field in 64-bit"
                        " mode and desynchronises; refused to chase —"
                        " LCCC keeps the requested prefix on the normal"
                        " row, the shortest valid encoding of the"
                        " prefixed request")
            return
        if invalid_who:
            row.note = ("invalid oracle bytes excluded from the"
                        f" comparison ({','.join(invalid_who)}: 2-byte"
                        " displacement field on a 66-prefixed near"
                        " branch)")
            ok_oracles = valid

        # Same fail-closed discipline for the REX.W imm32 sign-extension
        # miscompile (invalid_64_sext_imm32): an oracle form that stores
        # sext(imm32) where the request asked for the positive u32 is not
        # the requested program, whatever its length. The same bytes from
        # LCCC would be a codegen bug and are reported immediately. Unlike
        # the data16 partition this never empties the oracle set on its
        # own (the movabs forms stay valid), so there is no
        # all-invalid arm: if some day it would, the filter is skipped and
        # the differences get reported (fail closed, not laundered).
        if (row.lccc.ok and row.lccc.data is not None
                and invalid_64_sext_imm32(row.insn, row.lccc.data)):
            row.verdict = "WRONG-BYTES"
            row.note = ("LCCC emitted a REX.W imm32 form that"
                        " sign-extends where the request asked for the"
                        " positive u32 — a different program")
            return
        svalid = {k: v for k, v in ok_oracles.items()
                  if not invalid_64_sext_imm32(row.insn, v.data)}
        if 0 < len(svalid) < len(ok_oracles):
            row.note = (row.note + " | " if row.note else "") + (
                "oracle form(s) excluded: REX.W imm32 sign-extension"
                f" miscompile ({','.join(sorted(set(ok_oracles)-set(svalid)))})"
                " stores sext(imm32), not the requested u32"
                " (objdump-verified)")
            ok_oracles = svalid

    lengths = {k: len(v.data) for k, v in ok_oracles.items()}
    best = min(lengths.values())
    best_who = sorted(k for k, n in lengths.items() if n == best)
    best_bytes = sorted({v.data for v in ok_oracles.values() if len(v.data) == best})
    bytesets = {v.data for v in ok_oracles.values()}
    disagree = len(bytesets) > 1

    if not row.lccc.ok or row.lccc.data is None:
        row.verdict = "REJECTS-VALID"
        row.note = f"oracles accept ({','.join(sorted(ok_oracles))})"
        return

    n = len(row.lccc.data)
    if disagree:
        row.note = "oracles differ: " + ", ".join(
            f"{k}={len(v.data)}B" for k, v in sorted(ok_oracles.items()))
        if n < best:
            _classify_roundtrip(row, best_bytes, "BEATS", bits32=bits32,
                                seg_dead64=seg_fold)
        elif n == best:
            _classify_roundtrip(row, best_bytes, "ok-best", bits32=bits32,
                                seg_dead64=seg_fold)
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
            row.note += f" | shortest is {best}B from {','.join(best_who)}"
            _classify_roundtrip(row, best_bytes, "LONGER", bits32=bits32,
                                seg_dead64=seg_fold)
        return

    ref = next(iter(bytesets))
    if row.lccc.data == ref:
        row.verdict = "ok"
    elif is_declined_data16_relax(row.insn) and n > len(ref):
        # The fixed-rel16 policy (32-bit mode keeps `data16 je 1f` on the
        # rel16 row where GAS relaxes to `66 74 rel8`). A decline is only
        # honest after the round-trip proves the two forms are the same
        # program: an UNVERIFIED decline would mask a real mis-encoding
        # forever, so verification failure escalates instead.
        same = _roundtrip_same_as(row, [ref], bits32=bits32,
                                  seg_dead64=seg_fold)
        if same is True:
            row.verdict = "DECLINED-DATA16"
            if _DATA16_LOOP.match(row.insn):
                # Short-only rows (loop/jecxz): GAS DROPS the 66 prefix
                # with a warning where lccc keeps it. The forms are the
                # same program (66 only truncates IP on the taken
                # branch; the counter width is address-size selected),
                # but ONLY the round-trip above may say so -- arbitrary
                # different bytes on these rows stay WRONG-BYTES.
                note = (f"{len(ref)}B form from gas DROPS the 66 prefix"
                        " (warning: skipping prefixes on loop/jecxz)"
                        " leaving the plain short row; lccc keeps the"
                        " user's prefix -- 66 truncates IP to 16 bits"
                        " on the taken branch only, the counter width"
                        " is address-size selected (0x67), so both"
                        " forms are the same program for in-payload"
                        " targets")
            else:
                note = (f"{len(ref)}B form from gas relaxes the explicit"
                        " 16-bit displacement to the short row; refused"
                        " (fixed-rel16 policy, see prefix-words.casefile)"
                        " | round-trip verified against the relaxed form")
            row.note = (row.note + " | " if row.note else "") + note
        elif same is False:
            row.verdict = "WRONG-BYTES"
            row.note = (row.note + " | " if row.note else "") + (
                "LCCC disassembly differs from the relaxed oracle form"
                " (data16 row: decline cannot be claimed)")
        else:
            row.verdict = "UNVERIFIED-BYTES"
            row.note = (row.note + " | " if row.note else "") + (
                "objdump could not verify the data16 decline (fail-closed)")
    elif n < len(ref):
        row.note = (row.note + " | " if row.note else "") + (
            f"oracles agree on {len(ref)}B"
            f" ({','.join(sorted(ok_oracles))})")
        _classify_roundtrip(row, [ref], "BEATS", bits32=bits32,
                            seg_dead64=seg_fold)
    elif n > len(ref):
        row.note = (row.note + " | " if row.note else "") + (
            f"oracles agree on {len(ref)}B"
            f" ({','.join(sorted(ok_oracles))})")
        _classify_roundtrip(row, [ref], "LONGER", bits32=bits32,
                            seg_dead64=seg_fold)
    else:
        row.note = (row.note + " | " if row.note else "") + (
            f"same length, different bytes (oracle {ref.hex()})")
        _classify_roundtrip(row, [ref], "ok", bits32=bits32,
                            seg_dead64=seg_fold)


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


# An EXPLICIT `data16` branch names the 16-bit displacement form itself.
# In 32-bit mode lccc's fixed-rel16 policy keeps `data16 je 1f` on the
# rel16 near row even for a target GAS would relax down to the
# 66-prefixed SHORT form (3B); on short-only rows (loop/jrcxz) GAS DROPS
# the prefix with a warning where lccc keeps it. Both divergences
# are deliberate policies, classified separately so LONGER keeps its
# signal — and only after the round-trip proves equivalence (see
# classify). `call` is not here: it has no short row, so GAS never
# "relaxes" a data16 call — its truncated form is a hardware-invalid
# encoding handled by invalid_64_data16_branch instead.
_DECLINED_DATA16_RELAX = re.compile(
    r"^data16\s+(?:j[a-z]+|loop[a-z]*)\b", re.I)
# The LOOP/E0-E3 family: GAS drops the user's 66 prefix with a warning
# where lccc keeps it (the forms are the same program for in-payload
# targets -- 66 truncates IP only; the CX/ECX counter width is
# ADDRESS-SIZE selected, 0x67, never 66). Used only to pick the
# decline-note wording, never to bypass verification.
_DATA16_LOOP = re.compile(r"^data16\s+(?:loop[a-z]*|jcxz|jecxz)\b", re.I)


def is_declined_data16_relax(insn: str) -> bool:
    return _DECLINED_DATA16_RELAX.match(insn) is not None


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
    "UNVERIFIED-BEATS": 1,
    "UNVERIFIED-BYTES": 1,
    "REJECTS-VALID": 2,
    "LONGER": 3,
    "BEATS": 4,      # semantic round-trip verified
    "DISAGREE": 5,
    "ok-best": 6,
    "ok": 7,
    "DECLINED-FP": 7,   # shorter form exists but changes NaN payload
    "DECLINED-WRONG": 7, # shorter form exists but is not equivalent
    "DECLINED-DATA16": 7, # shorter form relaxes the requested 16-bit field
    "ORACLE-INVALID": 7,  # every oracle form is hardware-invalid; LCCC is right
    "both-reject": 8,
    "NO-ORACLE": 9,
}


def read_casefiles(paths: list[str]) -> list[str]:
    """Harvest positive instruction rows from asm-diff casefiles.

    `reject` groups are intentionally omitted: feeding expected failures to a
    batched remote assembler makes every containing batch fail and be split
    recursively, without adding useful encoding data.
    """
    out: list[str] = []
    seen: set[str] = set()
    for p in paths:
        in_reject_group = False
        for line in Path(p).read_text().splitlines():
            t = line.strip()
            if t.startswith(";;;"):
                fields = t[3:].split()
                in_reject_group = "reject" in fields[1:]
                continue
            if not t or t.startswith((";", "#", "//")):
                continue
            if in_reject_group or t.startswith(".") or t.endswith(":"):
                continue
            if t not in seen:
                seen.add(t)
                out.append(t)
    return out


def verdict_histogram(rows: list["Row"]) -> dict[str, int]:
    """Per-verdict row counts of a finished run (the aggregate record)."""
    counts: dict[str, int] = {}
    for r in rows:
        counts[r.verdict] = counts.get(r.verdict, 0) + 1
    return counts


def rows_digest(rows: list["Row"]) -> str:
    """A content digest of the row SET — identity, not verdicts.

    sha256 over the sorted comment-stripped row texts, each carrying its
    byte-exact pin when present. The histogram pins aggregate COUNTS; a
    compensating delete+add of same-verdict rows nets to zero and is
    invisible to it. This digest pins row identity: any row-set change —
    and any weakening of a row's byte-exact pin — is a baseline change
    that must be consciously re-recorded.
    """
    identities = sorted(
        r.insn.split("#")[0].strip()
        + ("" if r.byte_pin is None else f" @byte-exact={r.byte_pin.hex()}")
        for r in rows)
    return hashlib.sha256("\n".join(identities).encode()).hexdigest()


def check_verdict_histogram(rows: list["Row"], path: Path) -> bool:
    """Fail unless the run's per-verdict counts equal the recorded baseline.

    Counts only, never bytes: the baseline records what the corpus IS,
    not a byte-for-byte snapshot that would churn on every encoder
    improvement (BEATS staying BEATS through better bytes is fine). Row
    IDENTITY is pinned separately, by the `# rows-sha256:' digest line
    the baseline must carry: counts alone are a NET contract — a
    compensating delete+add of same-verdict rows nets to zero — so the
    digest closes that blind spot without churning on verdict-preserving
    byte improvements either (it covers the row set and the pins, not
    the verdicts).
    """
    try:
        text = path.read_text()
    except OSError as exc:
        # A missing/unreadable baseline is not drift, and "update the
        # baseline" would be misleading advice: there is nothing to
        # update. Say what actually happened.
        print(f"verdict histogram baseline {path}: unreadable ({exc}). "
              "A missing baseline is not drift — create it from the "
              "gate's own output in the same commit as the corpus change.",
              file=sys.stderr)
        return False
    expected: dict[str, int] = {}
    digest: str | None = None
    for line in text.splitlines():
        m = re.match(r"^#\s*rows-sha256:\s*([0-9a-f]{64})\s*$", line.strip(),
                     re.IGNORECASE)
        if m:
            if digest is not None:
                print(f"histogram baseline {path}: duplicate rows-sha256 "
                      "line — the digest is one line, not a "
                      "last-write-wins field", file=sys.stderr)
                return False
            digest = m.group(1)
            continue
        line = line.split("#")[0].strip()
        if not line:
            continue
        name, _, count = line.rpartition(" ")
        # A count line is `VERDICT <nonnegative integer>'. Everything
        # else is a malformed baseline, not data to silently overwrite:
        # a misspelled class is an unknown name (never discarded by the
        # nonzero projection), a negative count is not a count, and a
        # duplicate class is an ambiguity.
        if not count.isdigit():
            print(f"histogram baseline {path}: unparseable line {line!r} "
                  "(expected `VERDICT <nonnegative integer>')",
                  file=sys.stderr)
            return False
        if name not in SEVERITY:
            print(f"histogram baseline {path}: unknown verdict class "
                  f"{name!r} in line {line!r} (supported: "
                  f"{', '.join(SEVERITY)})", file=sys.stderr)
            return False
        if name in expected:
            print(f"histogram baseline {path}: duplicate verdict class "
                  f"{name!r} — every class is listed exactly once",
                  file=sys.stderr)
            return False
        expected[name] = int(count)
    if not expected:
        print(f"histogram baseline {path}: no verdict counts parsed — an "
              "empty baseline is not a contract. Record every verdict "
              "class (zeros included) plus the rows-sha256 digest, as the "
              "gate's own output spells them. Current digest: "
              f"{rows_digest(rows)}", file=sys.stderr)
        return False
    if digest is None:
        print(f"histogram baseline {path}: missing the `# rows-sha256:' "
              "digest line. Counts alone are a NET contract — a "
              "compensating delete+add of same-verdict rows nets to zero "
              "— so the baseline must also pin row identity. Add this "
              f"line: # rows-sha256: {rows_digest(rows)}", file=sys.stderr)
        return False
    missing_classes = [c for c in SEVERITY if c not in expected]
    if missing_classes:
        print(f"histogram baseline {path}: missing verdict classes "
              f"{missing_classes} — every class is listed exactly once, "
              "zeros included (a zero-recorded class is pinned absent)",
              file=sys.stderr)
        return False
    actual = verdict_histogram(rows)
    actual_digest = rows_digest(rows)
    # Compare through the nonzero projection on BOTH sides: a verdict class
    # recorded at 0 is documentation (the class is pinned absent) — the
    # actual dict simply has no key for it, and raw dict equality would
    # flag every zero row as drift. Anything APPEARING from zero, any count
    # change, and any class missing from the baseline still mismatches.
    counts_match = {k: v for k, v in expected.items() if v} == \
        {k: v for k, v in actual.items() if v}
    if counts_match and digest == actual_digest:
        return True
    print("verdict histogram drifted from the recorded baseline:", file=sys.stderr)
    for name in sorted(set(expected) | set(actual)):
        e, a = expected.get(name, 0), actual.get(name, 0)
        marker = "  " if e == a else "->"
        print(f"  {marker} {name:<18} {e:>4} {a:>4}", file=sys.stderr)
    if digest != actual_digest:
        print(f"  -> rows-sha256       recorded {digest}",
              file=sys.stderr)
        print(f"     rows-sha256       actual   {actual_digest}",
              file=sys.stderr)
    print("  update the baseline in the same commit that changed the corpus"
          " (the histogram is the aggregate coverage record, not a"
          " snapshot of bytes)", file=sys.stderr)
    return False


def main() -> int:
    global _OBJDUMP
    ap = argparse.ArgumentParser(
        description="LCCC vs GAS/Clang/ICX/GCC encoding differential.")
    ap.add_argument("--lccc", default=os.environ.get("LCCC", "./target/release/lccc"))
    ap.add_argument("--as", dest="gas", default=os.environ.get("LCCC_GAS", "as"))
    ap.add_argument("--objcopy", default=os.environ.get("LCCC_OBJCOPY", "objcopy"))
    # The disassembler decides BEATS/ok verdicts through decodes_same: it is
    # as much an oracle as `as`, and the corpus gates pin it (the runner
    # image's objdump is whatever binutils it ships — the exact unpinned-
    # tool class the 2.47 `as' pin exists to prevent).
    ap.add_argument("--objdump", default=os.environ.get("LCCC_OBJDUMP", "objdump"))
    ap.add_argument("--expect-histogram", default=None, metavar="FILE", type=Path,
                    help="fail unless the per-verdict row counts AND the "
                         "rows-sha256 row-identity digest equal this "
                         "recorded baseline: every verdict class exactly "
                         "once (zeros included), nonnegative integer "
                         "counts, one `# rows-sha256:' line. The digest "
                         "covers the row set and each row's byte-exact "
                         "pin — a compensating delete+add of same-verdict "
                         "rows fails on the digest alone")
    ap.add_argument("--insn", action="append", default=[],
                    help="one instruction (repeatable)")
    ap.add_argument("--file", action="append", default=[], metavar="FILE",
                    help="file with one instruction per line (repeatable)")
    ap.add_argument("--casefiles", nargs="*", default=[],
                    help="asm-diff casefiles to harvest positive instructions from (reject groups skipped)")
    ap.add_argument("--compiler", action="append", default=[],
                    help="extra Compiler Explorer id to use as an oracle")
    ap.add_argument("--offline", action="store_true",
                    help="local GNU as only; no network")
    ap.add_argument("--32", dest="bits32", action="store_true",
                    help="compare i686 code: LCCC/GAS --32, remote compilers -m32")
    ap.add_argument("--only", action="append", default=[],
                    help="report only these verdicts")
    ap.add_argument("--batch", type=int, default=60,
                    help="instructions per remote request (0 disables batching)")
    ap.add_argument("--max-report", type=int, default=200)
    ap.add_argument("--quiet", action="store_true")
    ap.add_argument("--json", type=Path,
                    help="write a per-instruction scoreboard (bytes vs every oracle)")
    args = ap.parse_args()
    _OBJDUMP = args.objdump

    insns: list[str] = list(args.insn)
    if args.file:
        for one in args.file:
            insns += [l.strip() for l in Path(one).read_text().splitlines()
                  if l.strip() and not l.strip().startswith("#")]
    if args.casefiles:
        insns += read_casefiles(args.casefiles)
    if not insns:
        ap.error("no instructions: use --insn, --file or --casefiles")
    if not args.bits32 and any(Path(p).parent.name == "i686"
                                for p in args.casefiles):
        ap.error("i686 casefiles require --32 (otherwise LCCC/GAS and remote "
                 "oracles use x86-64 mode)")

    # Byte-exact annotations are row contracts, not assembler input:
    # strip them from every source line (--insn, --file and casefile
    # harvest alike) before dedup, keeping the pinned bytes per stripped
    # text. A malformed or ambiguous annotation is a hard error (see
    # harvest_byte_pins for the policy).
    try:
        insns, byte_pins = harvest_byte_pins(insns)
    except ValueError as exc:
        ap.error(str(exc))

    seen: set[str] = set()
    uniq = [i for i in insns if not (i in seen or seen.add(i))]

    oracle_ids = dict(REMOTE_ORACLES)
    for c in args.compiler:
        oracle_ids[c] = c

    rows: list[Row] = []
    with tempfile.TemporaryDirectory(prefix="encdiff-") as td:
        tmp = Path(td)
        # Local assemblers are cheap; run them straight through.
        locals_lccc = [encode_local(args.lccc, i, tmp, args.objcopy,
                                    bits32=args.bits32) for i in uniq]
        locals_gas = [encode_local(args.gas, i, tmp, args.objcopy,
                                   bits32=args.bits32) for i in uniq]

        remote: dict[str, list[Encoding]] = {}
        if not args.offline:
            # Each oracle is an independent network stream, so fetch them
            # concurrently; within an oracle the instructions are batched.
            with concurrent.futures.ThreadPoolExecutor(
                    max_workers=max(1, len(oracle_ids))) as ex:
                futs = {
                    ex.submit(encode_remote_many, cid, uniq,
                              "-O0 -m32 -c" if args.bits32 else "-O0 -c",
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
            row = Row(insn, locals_lccc[k], byte_pin=byte_pins.get(insn))
            row.oracles["gas"] = locals_gas[k]
            for name, encs in remote.items():
                if k < len(encs):
                    row.oracles[name] = encs[k]
            classify(row, bits32=args.bits32)
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

    # Raw byte-length counts are useful for a size inventory, but are not the
    # semantic verdict: only a round-trip-verified row is a confirmed win.
    print("\n=== LCCC vs each oracle (raw payload lengths; not semantic verdicts) ===")
    print(f"{'oracle':<8} {'shorter':>7} {'tie':>6} {'longer':>7} {'n':>6}  "
          f"{'lccc B':>8} {'them B':>8}  delta")
    vs_rows = []
    for name in reachable:
        shorter = tie = longer = 0
        lccc_b = them_b = 0
        n = 0
        for r in rows:
            e = r.oracles.get(name)
            if not (e and e.ok and e.data is not None
                    and r.lccc.ok and r.lccc.data is not None):
                continue
            n += 1
            a, b = len(r.lccc.data), len(e.data)
            lccc_b += a
            them_b += b
            if a < b:
                shorter += 1
            elif a == b:
                tie += 1
            else:
                longer += 1
        delta = lccc_b - them_b
        vs_rows.append((name, shorter, tie, longer, n, lccc_b, them_b, delta))
        print(f"{name:<8} {shorter:>7} {tie:>6} {longer:>7} {n:>6}  "
              f"{lccc_b:>8} {them_b:>8}  {delta:+d}")

    if args.json:
        payload = {
            "schema": 4,
            "target": "i686" if args.bits32 else "x86_64",
            "n": len(rows),
            "verdicts": counts,
            "oracles_reached": reachable,
            "vs": [
                {"oracle": n, "shorter": s, "tie": t, "longer": l, "n": nn,
                 "lccc_bytes": lb, "oracle_bytes": ob, "delta": d}
                for (n, s, t, l, nn, lb, ob, d) in vs_rows
            ],
            "rows": [
                {
                    "insn": r.insn,
                    "verdict": r.verdict,
                    "note": r.note,
                    "lccc": None if not r.lccc.ok else r.lccc.data.hex(),
                    "lccc_n": None if not r.lccc.ok else len(r.lccc.data),
                    "oracles": {
                        k: None if not e.ok else e.data.hex()
                        for k, e in r.oracles.items()
                    },
                    "oracle_n": {
                        k: None if not (e.ok and e.data is not None) else len(e.data)
                        for k, e in r.oracles.items()
                    },
                }
                for r in rows
            ],
        }
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps(payload, indent=2) + "\n")
        print(f"wrote {args.json}")

    bad = sum(counts.get(k, 0) for k in (
        "WRONG-BYTES", "UNVERIFIED-BEATS", "UNVERIFIED-BYTES",
        "REJECTS-VALID", "LONGER"))
    if bad:
        return 1
    if args.expect_histogram is not None:
        if not check_verdict_histogram(rows, args.expect_histogram):
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
