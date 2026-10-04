#!/usr/bin/env python3
"""
aarch64_encoder_differential.py
===============================

Differential test of LCCC's AArch64 instruction encoder against GNU binutils.

Why this exists
---------------
Hand-written encoder tests share a fatal flaw with the corpus they are derived
from: they can only check the operand shapes somebody *thought* to write down.
The upstream `claudes-c-compiler` fork accumulated ~330 AArch64 encoder bug
reports and ~87 000 lines of property tests, and every single one of those was
a guess at what an encoder *ought* to do.

This tool takes the opposite approach: it does not have an opinion about AArch64
at all. It generates **uniformly random 32-bit words**, asks GNU `objdump` which
of them decode to real instructions, and then feeds the resulting canonical
assembly text to LCCC's built-in assembler and to GNU `as` in parallel. Any
difference in the emitted bytes is a bug *by construction* -- the oracle is
binutils, not a human.

Because the generator is the decoder of a second, independent implementation,
it needs no per-mnemonic grammar, no hand-written operand templates, and no
maintenance as the ISA grows. It reaches SVE, SME, AdvSIMD, the predicated and
scaled forms and all the odd system encodings on the first run, in proportion to
how much of the encoding space they occupy.

Method
------
1.  **Generate.** Emit N random 32-bit words as a raw binary blob.

2.  **Decode.** `objdump -D -b binary -m aarch64 -w` over the blob. Words that
    do not decode print as `.word`/`.inst` and are discarded; the rest yield
    `(address, encoding, mnemonic, operands)`.

3.  **Pin.** Every surviving instruction is re-emitted at *exactly* its original
    address using `.org` for the gaps. This matters more than it looks:
    `objdump` renders PC-relative operands (`bl`, `adr`, `adrp`, `b.cond`,
    `tbz`, `cbz`, `ldr literal`) as **absolute** targets derived from the
    instruction's address. Re-assembling that text at a different address
    changes the required displacement and can push it out of range, which would
    make the instruction unencodable and silently destroy coverage for the
    entire PC-relative family. Pinning the address makes every displacement
    exactly reproducible, so branch and literal forms are covered as well as
    everything else. (Verified: LCCC's `.org` zero-fills exactly like GAS, so
    the padding itself is not a source of divergence.)

    The cost of pinning is that instructions are generally **not** at
    `4 * index` once a subset has been selected: `.org` inserts padding, so the
    byte offset of instruction `k` is `items[k].addr`, not `4 * k`. Slicing at
    `4 * k` silently compares padding bytes instead of machine code, which
    reports a reassuring "everything matches" while testing almost nothing.
    This tool slices at the pinned address, and the round-trip canary below
    makes that class of mistake impossible to miss.

4.  **Differ.** The pinned source is assembled by GAS and by LCCC, and the
    `.text` bytes are compared at each instruction's **pinned address**.
    Batches that fail to assemble -- because one instruction in the batch is
    unsupported -- are recursively bisected so that a single bad apple costs
    `log2(batch)` extra invocations rather than the whole batch.

The round-trip canary
---------------------
For every instruction, GAS's reassembled bytes at the pinned address are
compared against the **original random word** that objdump decoded. They should
be identical: same address, same instruction, so same encoding.

This single check validates the whole pipeline -- objdump parsing, address
pinning, `.org` behaviour, and byte extraction -- on every single instruction
tested. It is the check whose absence let an earlier revision of this tool
report "0 mismatches" while comparing padding. If the canary fails for an
instruction, that instruction is reported as `ROUNDTRIP_DRIFT` and excluded
from the correctness comparison rather than silently counted as a pass.

Result classes
--------------
`MISENCODE`        round trip verified, GAS and LCCC disagree   <-- a real bug
`ROUNDTRIP_DRIFT`  GAS did not reproduce the decoded word       <-- excluded
`LCCC_REJECT`      GAS accepted it, LCCC refused it             <-- coverage gap
`GAS_REJECT`       LCCC accepted it, GAS refused it             <-- possibly intentional
`BOTH_REJECT`      neither accepted it (objdump/GAS disagree, e.g. ISA level)
`OK`               round trip verified and byte-identical

Only `MISENCODE` is unambiguously a defect; the tool exits non-zero on those and
reports the rest as information.

Self-test
---------
A differential harness that silently stops working reports "0 mismatches"
forever, which is the most dangerous possible failure mode for this class of
tool. `self_test()` therefore asserts, before doing anything else:

1. a known-good sequence encodes identically under both assemblers;
2. a known-bad mnemonic is rejected by both (proving it can detect junk);
3. **sparse-address extraction**: two instructions pinned at 0x0 and 0x100 are
   recovered from *both* objects at those addresses, and the padding at 0x4 is
   proven *not* to be the second instruction -- the exact regression that once
   made this harness compare padding against padding;
4. an `objdump` canary: a known raw word decodes to a known mnemonic, proving
   the decoder/parser half of the pipeline is alive.

Run it alone with `--self-test-only`.

Usage
-----
    python3 scripts/aarch64_encoder_differential.py --n 200000
    python3 scripts/aarch64_encoder_differential.py --n 20000 -o report.json
    python3 scripts/aarch64_encoder_differential.py --self-test-only

Requires `binutils-aarch64-linux-gnu` (or any aarch64 GAS/objdump/objcopy
triple) and a built LCCC binary.

Note on scratch space: every intermediate `.s`/`.o`/`.bin` file is deleted as
soon as it has been read. `finally`-based cleanup cannot help against SIGKILL,
so this tool does not rely on it -- it never accumulates more than one batch's
worth of files in the first place.
"""

from __future__ import annotations

import argparse
import json
import os
import random
import re
import shutil
import subprocess
import sys
import tempfile
from collections import Counter
from dataclasses import dataclass
from pathlib import Path

# --------------------------------------------------------------------------
# Result classes
# --------------------------------------------------------------------------

OK = "OK"
MISENCODE = "MISENCODE"
ROUNDTRIP_DRIFT = "ROUNDTRIP_DRIFT"
LCCC_REJECT = "LCCC_REJECT"
GAS_REJECT = "GAS_REJECT"
BOTH_REJECT = "BOTH_REJECT"

PROBLEM_CLASSES = (MISENCODE,)

# objdump -w output:   "     1c:\tb200c4c3 \torr\tx3, x6, #0x303030303030303"
INSN_RE = re.compile(r"^\s*([0-9a-fA-F]+):\s+([0-9a-fA-F]{8})\s+(\S+)\s*(.*)$")

# Known-good / known-bad fixtures for the self-test.
#
# `add x0, x1, x2` == word 0x8b020020, `sub x3, x4, x5` == word 0xcb050083.
#
# The two spellings matter and are kept separate on purpose: `objdump` renders
# the instruction WORD big-endian, while the bytes in a `.text` section are
# little-endian. Comparing one against the other without converting is a
# silent, total failure -- every instruction would look like a mismatch (or,
# worse, a match if the check were inverted).
SELFTEST_A_WORD = "8b020020"      # objdump's rendering
SELFTEST_B_WORD = "cb050083"      # objdump's rendering
SELFTEST_A_BYTES = "2000028b"     # little-endian section bytes
SELFTEST_B_BYTES = "830005cb"     # little-endian section bytes
SELFTEST_A_TEXT = "add x0, x1, x2"
SELFTEST_B_TEXT = "sub x3, x4, x5"
SELFTEST_SPARSE_ADDR = 0x100


def word_to_le_bytes(word: str) -> str:
    """Convert objdump's big-endian instruction word to little-endian bytes.

    `objdump` prints `8b020020` for the bytes `20 00 02 8b`. The round-trip
    canary compares a decoded word against section bytes, so this conversion is
    on the critical path of every single instruction the tool reports on.
    """
    return bytes.fromhex(word)[::-1].hex()


class HarnessError(RuntimeError):
    """The differential harness itself is broken; refuse to report results."""


@dataclass
class AsmResult:
    ok: bool
    data: bytes = b""
    stderr: str = ""

    @property
    def first_error(self) -> str:
        for line in self.stderr.splitlines():
            if "error" in line.lower():
                return line.strip()
        return self.stderr.strip().splitlines()[0] if self.stderr.strip() else "unknown"


@dataclass
class Finding:
    status: str
    addr: int
    text: str
    mnemonic: str
    encoding: str          # the 8-hex-digit word GNU objdump decoded
    gas_bytes: str | None = None
    lccc_bytes: str | None = None
    gas_error: str = ""
    lccc_error: str = ""


@dataclass
class Toolchain:
    """Locates and drives the assemblers. LCCC selects its target from argv[0],
    so the binary is exposed under an `aarch64-...-ccc` name in a temp dir."""

    gas: str
    objcopy: str
    objdump: str
    lccc_link: Path
    tmpdir: Path
    objdump_arch: str
    prologue: str

    # -- discovery ---------------------------------------------------------

    @staticmethod
    def _find_tool(triple: str, name: str, env: str | None) -> str:
        if env:
            return env
        for cand in (f"{triple}-{name}", f"{triple}-linux-gnu-{name}", name):
            p = shutil.which(cand)
            if p:
                return p
        raise HarnessError(
            f"cannot find {name}. Install it (Debian: "
            f"`apt-get install binutils-aarch64-linux-gnu`) or pass --{name}."
        )

    @staticmethod
    def _find_lccc(explicit: str | None) -> Path:
        if explicit:
            p = Path(explicit)
            if not p.is_file():
                raise HarnessError(f"--lccc points at {p}, which is not a file")
            return p.resolve()

        here = Path(__file__).resolve().parent.parent
        candidates = [
            here / "target" / "fastbuild" / "lccc",
            here / "target" / "debug" / "lccc",
            here / "target" / "release" / "lccc",
        ]
        env_dir = os.environ.get("CARGO_TARGET_DIR")
        if env_dir:
            candidates.insert(0, Path(env_dir) / "fastbuild" / "lccc")
            candidates.insert(1, Path(env_dir) / "target" / "fastbuild" / "lccc")
        for c in candidates:
            if c.is_file():
                return c.resolve()
        w = shutil.which("lccc")
        if w:
            return Path(w).resolve()
        raise HarnessError(
            "cannot find an LCCC binary. Build it with ./scripts/build_lccc_fast.sh "
            "or pass --lccc /path/to/lccc."
        )

    # -- construction ------------------------------------------------------

    @classmethod
    def build(cls, args: argparse.Namespace) -> "Toolchain":
        triple = args.triple
        gas = cls._find_tool(triple, "as", args.gas)
        objcopy = cls._find_tool(triple, "objcopy", args.objcopy)
        objdump = cls._find_tool(triple, "objdump", args.objdump)

        tmpdir = Path(tempfile.mkdtemp(prefix="lccc-a64diff-"))
        # LCCC picks its backend from argv[0] (see src/driver/cli.rs): the
        # binary must be *named* aarch64-*-ccc to assemble AArch64 at all.
        link = tmpdir / f"{triple}-linux-gnu-ccc"
        link.symlink_to(cls._find_lccc(args.lccc))

        return cls(
            gas=gas,
            objcopy=objcopy,
            objdump=objdump,
            lccc_link=link,
            tmpdir=tmpdir,
            objdump_arch=args.objdump_arch,
            prologue=args.prologue,
        )

    def close(self) -> None:
        shutil.rmtree(self.tmpdir, ignore_errors=True)

    # -- primitives --------------------------------------------------------

    def _run(self, argv: list[str]) -> subprocess.CompletedProcess:
        return subprocess.run(argv, capture_output=True, text=True, timeout=180)

    def assemble(self, src: str, tag: str, use_lccc: bool) -> AsmResult:
        """Assemble `src` and return the raw `.text` bytes.

        Every intermediate file is unlinked as soon as it has been consumed.
        A run performs tens of thousands of these calls, so leaving files behind
        for a `finally` to collect would needlessly hold the whole run's worth
        of scratch -- and `finally` does not run at all under SIGKILL.
        """
        d = self.tmpdir
        s, o, b = d / f"{tag}.s", d / f"{tag}.o", d / f"{tag}.bin"
        s.write_text(self.prologue + src)
        try:
            if use_lccc:
                r = self._run([str(self.lccc_link), "-c", str(s), "-o", str(o)])
            else:
                r = self._run([self.gas, str(s), "-o", str(o)])
            if r.returncode != 0 or not o.is_file():
                return AsmResult(False, b"", (r.stderr or "") + (r.stdout or ""))

            # Extract the raw .text bytes so the comparison is on machine code,
            # not on ELF layout or symbol tables.
            rc = self._run(
                [self.objcopy, "-O", "binary", "--only-section=.text", str(o), str(b)]
            )
            if rc.returncode != 0 or not b.is_file():
                return AsmResult(False, b"", (rc.stderr or "") + (rc.stdout or ""))
            return AsmResult(True, b.read_bytes(), "")
        finally:
            for p in (s, o, b):
                p.unlink(missing_ok=True)

    # -- self-test ---------------------------------------------------------

    def self_test(self) -> None:
        """Prove the harness can detect a difference before trusting its silence.

        A differential tester whose oracle has quietly stopped working reports
        zero mismatches for the rest of its life. These assertions prevent that.
        """
        # 1. a valid sequence must agree, and must be the length we expect.
        good = f".text\n{SELFTEST_A_TEXT}\n{SELFTEST_B_TEXT}\n"
        g = self.assemble(good, "selftest_good", use_lccc=False)
        l = self.assemble(good, "selftest_good", use_lccc=True)
        if not (g.ok and l.ok):
            raise HarnessError(
                "self-test failed: a valid instruction did not assemble\n"
                f"  gas : {g.first_error}\n  lccc: {l.first_error}"
            )
        if g.data != l.data:
            raise HarnessError(
                "self-test failed: GAS and LCCC disagree on a contiguous pair "
                f"({g.data.hex()} vs {l.data.hex()}) -- harness is misconfigured"
            )
        if g.data.hex() != SELFTEST_A_BYTES + SELFTEST_B_BYTES:
            raise HarnessError(
                f"self-test failed: expected {SELFTEST_A_BYTES}{SELFTEST_B_BYTES}, "
                f"got {g.data.hex()} -- byte extraction is broken"
            )

        # 2. an unknown mnemonic must be rejected by both sides.
        #    NB: `add x0, x1, x2, x3` is NOT usable here -- LCCC accepts surplus
        #    operands and silently ignores them while GAS rejects them (a real
        #    defect, recorded separately). Use a token neither side can parse.
        bad = ".text\nfrobnicate x0, x1\n"
        g = self.assemble(bad, "selftest_bad", use_lccc=False)
        l = self.assemble(bad, "selftest_bad", use_lccc=True)
        if g.ok or l.ok:
            raise HarnessError(
                "self-test failed: an invalid instruction was accepted "
                f"(gas ok={g.ok}, lccc ok={l.ok}) -- harness cannot detect junk"
            )

        # 3. sparse-address extraction. Two instructions pinned at 0x0 and 0x100
        #    must be recoverable at *those* addresses. This is the regression
        #    test for slicing at `4 * index`: the padding at 0x4 must NOT be the
        #    second instruction, otherwise the test could pass while comparing
        #    padding against padding.
        sparse = (
            f".text\n{SELFTEST_A_TEXT}\n"
            f".org 0x{SELFTEST_SPARSE_ADDR:x}\n{SELFTEST_B_TEXT}\n"
        )
        g = self.assemble(sparse, "selftest_sparse", use_lccc=False)
        l = self.assemble(sparse, "selftest_sparse", use_lccc=True)
        if not (g.ok and l.ok):
            raise HarnessError(
                "self-test failed: sparse `.org` layout did not assemble\n"
                f"  gas : {g.first_error}\n  lccc: {l.first_error}"
            )
        need = SELFTEST_SPARSE_ADDR + 4
        for name, res in (("gas", g), ("lccc", l)):
            if len(res.data) < need:
                raise HarnessError(
                    f"self-test failed: {name} produced {len(res.data)} bytes, "
                    f"expected at least {need} for the sparse layout"
                )
            if res.data[0:4].hex() != SELFTEST_A_BYTES:
                raise HarnessError(
                    f"self-test failed: {name} instruction at 0x0 is "
                    f"{res.data[0:4].hex()}, expected {SELFTEST_A_BYTES}"
                )
            got = res.data[SELFTEST_SPARSE_ADDR:SELFTEST_SPARSE_ADDR + 4].hex()
            if got != SELFTEST_B_BYTES:
                raise HarnessError(
                    f"self-test failed: {name} instruction at "
                    f"0x{SELFTEST_SPARSE_ADDR:x} is {got}, expected "
                    f"{SELFTEST_B_BYTES} -- pinned-address extraction is broken"
                )
            # The trap: if we had sliced at 4*index we would read padding here.
            if res.data[4:8].hex() == SELFTEST_B_BYTES:
                raise HarnessError(
                    f"self-test failed: {name} placed the second instruction at "
                    "offset 4 despite `.org`; the sparse layout is not exercised"
                )
        if g.data[SELFTEST_SPARSE_ADDR:SELFTEST_SPARSE_ADDR + 4] != \
           l.data[SELFTEST_SPARSE_ADDR:SELFTEST_SPARSE_ADDR + 4]:
            raise HarnessError("self-test failed: sparse-layout bytes differ")

        # 4. objdump canary: a known raw word must decode to a known mnemonic,
        #    proving the decode/parse half of the pipeline is alive.
        blob = self.tmpdir / "selftest_canary.bin"
        blob.write_bytes(bytes.fromhex(SELFTEST_A_WORD)[::-1])  # word -> LE bytes
        r = self._run(
            [self.objdump, "-D", "-b", "binary", "-m", self.objdump_arch,
             "-w", str(blob)]
        )
        blob.unlink(missing_ok=True)
        if r.returncode != 0:
            raise HarnessError(f"self-test failed: objdump failed: {r.stderr.strip()}")
        parsed = [m for m in (INSN_RE.match(ln) for ln in r.stdout.splitlines()) if m]
        if not parsed or parsed[0].group(2).lower() != SELFTEST_A_WORD:
            raise HarnessError(
                "self-test failed: objdump did not report the canary word "
                f"{SELFTEST_A_WORD} -- decoding/parsing pipeline is broken"
            )
        if parsed[0].group(3) != "add":
            raise HarnessError(
                f"self-test failed: canary word decoded to `{parsed[0].group(3)}`, "
                "expected `add`"
            )


# --------------------------------------------------------------------------
# Corpus generation
# --------------------------------------------------------------------------


@dataclass
class Insn:
    addr: int
    encoding: str
    mnemonic: str
    operands: str

    @property
    def text(self) -> str:
        return f"{self.mnemonic} {self.operands}".rstrip()


def generate_corpus(tc: Toolchain, n_words: int, seed: int) -> list[Insn]:
    """Random 32-bit words -> GNU's canonical disassembly of the valid ones."""
    rng = random.Random(seed)
    blob = tc.tmpdir / "random.bin"
    try:
        blob.write_bytes(rng.randbytes(4 * n_words))
        r = tc._run(
            [tc.objdump, "-D", "-b", "binary", "-m", tc.objdump_arch, "-w", str(blob)]
        )
    finally:
        blob.unlink(missing_ok=True)
    if r.returncode != 0:
        raise HarnessError(f"objdump failed: {r.stderr.strip()}")

    out: list[Insn] = []
    for line in r.stdout.splitlines():
        m = INSN_RE.match(line)
        if not m:
            continue
        addr, enc, mn, ops = m.groups()
        if mn.startswith("."):      # .word / .inst / .long => did not decode
            continue
        out.append(
            Insn(
                addr=int(addr, 16),
                encoding=enc.lower(),
                mnemonic=mn,
                operands=ops.strip(),
            )
        )
    return out


def select_corpus(
    corpus: list[Insn], per_mnemonic: int, max_insns: int
) -> tuple[list[Insn], dict[str, int]]:
    """Cap how many distinct texts we test per mnemonic.

    Uniform random words heavily over-sample the dense parts of the encoding
    space (a handful of mnemonics would otherwise be tested thousands of times
    while narrow encodings never appear). Capping per mnemonic spreads a fixed
    test budget across the ISA instead of spending it all on `add`.

    The result is intentionally **sparse**: selected instructions keep their
    original addresses, so consecutive picked instructions are usually far apart
    and the gaps are filled by `.org`. Any consumer that slices output by index
    rather than by address will silently read padding.
    """
    seen: set[str] = set()
    counts: Counter = Counter()
    picked: list[Insn] = []
    for ins in corpus:
        if counts[ins.mnemonic] >= per_mnemonic:
            continue
        key = ins.text
        if key in seen:
            continue
        seen.add(key)
        counts[ins.mnemonic] += 1
        picked.append(ins)
        if len(picked) >= max_insns:
            break
    return picked, dict(counts)


# --------------------------------------------------------------------------
# The differ
# --------------------------------------------------------------------------


class Differ:
    def __init__(self, tc: Toolchain, batch: int, verbose: bool):
        self.tc = tc
        self.batch = batch
        self.verbose = verbose
        self.calls = 0
        self._seq = 0

    def _src(self, items: list[Insn]) -> str:
        """Emit `items`, pinned to their original addresses via `.org`."""
        parts = [".text\n"]
        cursor = 0
        for ins in items:
            if ins.addr > cursor:
                parts.append(f".org 0x{ins.addr:x}\n")
                cursor = ins.addr
            elif ins.addr < cursor:
                raise HarnessError(
                    f"non-monotonic address 0x{ins.addr:x} after 0x{cursor:x}"
                )
            parts.append(ins.text + "\n")
            cursor += 4
        return "".join(parts)

    def _batch(self, items: list[Insn], depth: int) -> list[Finding]:
        self._seq += 1
        self.calls += 2
        tag = f"b{self._seq}"
        src = self._src(items)
        g = self.tc.assemble(src, tag + "_g", use_lccc=False)
        l = self.tc.assemble(src, tag + "_l", use_lccc=True)
        # `.org` stretches the section out to the highest pinned address, so the
        # required length is driven by the last *address*, not by the count.
        need = items[-1].addr + 4

        if g.ok and l.ok and len(g.data) >= need and len(l.data) >= need:
            findings = []
            for ins in items:
                gb = g.data[ins.addr:ins.addr + 4]
                lb = l.data[ins.addr:ins.addr + 4]
                if gb.hex() != word_to_le_bytes(ins.encoding):
                    # The canary. GAS reassembling the text at the same address
                    # must reproduce the word objdump decoded. If it does not,
                    # the decode/pin/extract pipeline is not trustworthy for
                    # this instruction, so it must not count as a pass.
                    findings.append(
                        Finding(ROUNDTRIP_DRIFT, ins.addr, ins.text, ins.mnemonic,
                                ins.encoding, gb.hex(), lb.hex())
                    )
                elif gb != lb:
                    findings.append(
                        Finding(MISENCODE, ins.addr, ins.text, ins.mnemonic,
                                ins.encoding, gb.hex(), lb.hex())
                    )
                else:
                    findings.append(
                        Finding(OK, ins.addr, ins.text, ins.mnemonic, ins.encoding,
                                gb.hex(), lb.hex())
                    )
            return findings

        if len(items) == 1:
            ins = items[0]
            if not g.ok and not l.ok:
                status = BOTH_REJECT
            elif not g.ok:
                status = GAS_REJECT
            elif not l.ok:
                status = LCCC_REJECT
            else:
                status = MISENCODE
            return [
                Finding(
                    status, ins.addr, ins.text, ins.mnemonic, ins.encoding,
                    g.data.hex() if g.ok else None,
                    l.data.hex() if l.ok else None,
                    "" if g.ok else g.first_error,
                    "" if l.ok else l.first_error,
                )
            ]

        # Something in the batch is unsupported: bisect so one bad instruction
        # costs log2(batch) extra invocations, not the whole batch.
        mid = len(items) // 2
        return self._batch(items[:mid], depth + 1) + self._batch(items[mid:], depth + 1)

    def run(self, items: list[Insn]) -> list[Finding]:
        findings: list[Finding] = []
        total = len(items)
        for i in range(0, total, self.batch):
            chunk = items[i:i + self.batch]
            findings.extend(self._batch(chunk, 0))
            done = min(i + self.batch, total)
            if self.verbose and (done % (self.batch * 10) == 0 or done == total):
                print(f"  ... {done}/{total} instructions", file=sys.stderr, flush=True)
        return findings


# --------------------------------------------------------------------------
# Reporting
# --------------------------------------------------------------------------


def differing_bits(a: str, b: str) -> str:
    try:
        ia, ib = int(a, 16), int(b, 16)
    except ValueError:
        return ""
    x = ia ^ ib
    return "".join(str((x >> i) & 1) for i in range(31, -1, -1))


def load_lccc_mnemonics(path: str | None) -> set[str]:
    if not path or not Path(path).is_file():
        return set()
    s = Path(path).read_text()
    toks = set(re.findall(r'^\s*"([a-z0-9_.]+)"\s*(?:\||=>)', s, re.M))
    toks |= set(re.findall(r'"([a-z0-9_.]{2,})"\s*=>', s))
    return {t for t in toks if re.fullmatch(r"[a-z][a-z0-9_.]*", t)}


def report(findings: list[Finding], lccc_mnemonics: set[str], args, stats: dict) -> int:
    by_status = Counter(f.status for f in findings)
    mis = [f for f in findings if f.status == MISENCODE]
    drift = [f for f in findings if f.status == ROUNDTRIP_DRIFT]
    verified = by_status.get(OK, 0)

    print()
    print("=" * 78)
    print("LCCC AArch64 encoder differential vs GNU binutils")
    print("=" * 78)
    print(f"  seed ............ {args.seed}")
    print(f"  random words .... {args.n:,}")
    print(f"  decoded ......... {stats['decoded']:,} ({stats['decode_rate']:.1%} of encoding space)")
    print(f"  distinct texts .. {stats['distinct']:,}")
    print(f"  tested .......... {len(findings):,} across {stats['mnemonics']} mnemonics")
    print(f"  assembler calls . {stats['calls']:,}")
    print()
    print("  result                 count")
    print("  " + "-" * 46)
    for k in (OK, MISENCODE, ROUNDTRIP_DRIFT, LCCC_REJECT, GAS_REJECT, BOTH_REJECT):
        if by_status.get(k):
            print(f"  {k:<20} {by_status[k]:>8,}")
    print()

    if drift:
        by_mn = Counter(f.mnemonic for f in drift)
        print(f"  {len(drift):,} instruction(s) where GAS did not reproduce the word "
              f"objdump decoded.")
        print("  These are EXCLUDED from the correctness comparison -- the "
              "decode/pin/reassemble")
        print("  round trip is not verified for them. Common causes: objdump "
              "printing an alias")
        print("  GAS canonicalises differently, or an ISA level objdump decodes "
              "but GAS cannot")
        print("  assemble. They are reported, not skipped silently.")
        print("    " + ", ".join(f"{m}({c})" for m, c in by_mn.most_common(12)))
        print()

    if mis:
        print(f"!!! {len(mis)} MISENCODING(S) !!!")
        print("-" * 78)
        for f in mis[:60]:
            print(f"  {f.text}")
            print(f"    @0x{f.addr:x}  objdump word {f.encoding}")
            print(f"    GAS   {f.gas_bytes}")
            print(f"    LCCC  {f.lccc_bytes}")
            xor = differing_bits(f.gas_bytes or "0", f.lccc_bytes or "0")
            if xor:
                print(f"    diff  {xor}  ({bin(int(xor, 2)).count('1')} bits)")
        if len(mis) > 60:
            print(f"  ... and {len(mis) - 60} more (see JSON report)")
        by_mn = Counter(f.mnemonic for f in mis)
        print()
        print("  worst mnemonics: " + ", ".join(f"{m}({c})" for m, c in by_mn.most_common(15)))
    else:
        print(f"No misencodings: LCCC's AArch64 encoder is byte-identical to GNU as "
              f"on all {verified:,} round-trip-verified")
        print("instructions tested.")

    rejected = [f for f in findings if f.status == LCCC_REJECT]
    if rejected:
        by_mn = Counter(f.mnemonic for f in rejected)
        print()
        print(f"  {len(rejected):,} instruction(s) GAS accepts and LCCC rejects "
              f"(coverage gap, not a misencoding):")
        print("    " + ", ".join(f"{m}({c})" for m, c in by_mn.most_common(20)))
        for f in rejected[:10]:
            print(f"      {f.text}")
            print(f"        lccc: {f.lccc_error[:120]}")

    if lccc_mnemonics:
        hit = {f.mnemonic for f in findings} & lccc_mnemonics
        miss = sorted(lccc_mnemonics - hit)
        print()
        print(f"  LCCC mnemonic coverage: {len(hit)}/{len(lccc_mnemonics)} "
              f"({len(hit) / len(lccc_mnemonics):.1%})")
        if miss:
            print(f"  never sampled ({len(miss)}): {' '.join(miss[:60])}"
                  + (" ..." if len(miss) > 60 else ""))
    print()
    return 1 if mis else 0


# --------------------------------------------------------------------------
# Entry point
# --------------------------------------------------------------------------


def positive_int(name: str, value: int) -> int:
    """Fail closed: a nonpositive limit silently yields an empty corpus, which
    would let the run report 'zero mismatches' having tested nothing."""
    if value <= 0:
        raise HarnessError(f"--{name} must be positive, got {value}")
    return value


def main() -> int:
    ap = argparse.ArgumentParser(
        description="Differential-test LCCC's AArch64 encoder against GNU binutils.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    ap.add_argument("--n", type=int, default=200_000,
                    help="random 32-bit words to generate (default: 200000)")
    ap.add_argument("--seed", type=int, default=1, help="PRNG seed (default: 1)")
    ap.add_argument("--batch", type=int, default=64,
                    help="instructions per assembler call (default: 64)")
    ap.add_argument("--per-mnemonic", type=int, default=64,
                    help="max distinct texts tested per mnemonic (default: 64)")
    ap.add_argument("--max-insns", type=int, default=30_000,
                    help="hard cap on instructions tested (default: 30000)")
    ap.add_argument("--triple", default="aarch64",
                    help="binutils triple prefix (default: aarch64)")
    ap.add_argument("--objdump-arch", default="aarch64",
                    help="-m value for objdump (default: aarch64)")
    ap.add_argument("--prologue", default="",
                    help="assembly prepended to every generated file")
    ap.add_argument("--lccc", help="path to the LCCC binary (default: auto-detect)")
    ap.add_argument("--gas", help="path to the aarch64 assembler")
    ap.add_argument("--objcopy", help="path to objcopy")
    ap.add_argument("--objdump", help="path to objdump")
    ap.add_argument("--lccc-mnemonics",
                    default="src/backend/arm/assembler/encoder/mod.rs",
                    help="source file to scrape LCCC's mnemonic list from")
    ap.add_argument("-o", "--json", help="write a machine-readable report here")
    ap.add_argument("--self-test-only", action="store_true",
                    help="run the harness self-tests and exit")
    ap.add_argument("-v", "--verbose", action="store_true")
    args = ap.parse_args()

    tc = Toolchain.build(args)
    try:
        tc.self_test()
        if args.self_test_only or args.verbose:
            print("self-test: harness verified (agreement, junk rejection, "
                  "sparse-address extraction, objdump canary)", file=sys.stderr)
        if args.self_test_only:
            return 0

        for name in ("n", "batch", "per-mnemonic", "max-insns"):
            positive_int(name, getattr(args, name.replace("-", "_")))

        corpus = generate_corpus(tc, args.n, args.seed)
        if not corpus:
            raise HarnessError(
                f"objdump decoded 0 instructions from {args.n} random words; "
                "refusing to report a result from an empty corpus"
            )
        distinct = len({i.text for i in corpus})
        if args.verbose:
            print(f"decoded {len(corpus):,} instructions ({distinct:,} distinct)",
                  file=sys.stderr)

        picked, _ = select_corpus(corpus, args.per_mnemonic, args.max_insns)
        if not picked:
            raise HarnessError(
                "instruction selection produced an empty corpus "
                f"(per-mnemonic={args.per_mnemonic}, max-insns={args.max_insns}); "
                "refusing to report a result from an empty corpus"
            )

        differ = Differ(tc, args.batch, args.verbose)
        findings = differ.run(picked)
    finally:
        tc.close()

    stats = {
        "decoded": len(corpus),
        "decode_rate": len(corpus) / max(1, args.n),
        "distinct": distinct,
        "mnemonics": len({f.mnemonic for f in findings}),
        "calls": differ.calls,
    }
    rc = report(findings, load_lccc_mnemonics(args.lccc_mnemonics), args, stats)

    if args.json:
        Path(args.json).write_text(
            json.dumps(
                {
                    "args": {k: v for k, v in vars(args).items()},
                    "stats": stats,
                    "findings": [
                        {
                            "status": f.status, "addr": f.addr, "text": f.text,
                            "mnemonic": f.mnemonic, "encoding": f.encoding,
                            "gas": f.gas_bytes, "lccc": f.lccc_bytes,
                            "gas_error": f.gas_error, "lccc_error": f.lccc_error,
                        }
                        for f in findings
                        if f.status != OK
                    ],
                },
                indent=2,
            )
        )
        print(f"report written to {args.json}")

    return rc


if __name__ == "__main__":
    try:
        sys.exit(main())
    except HarnessError as e:
        print(f"error: {e}", file=sys.stderr)
        sys.exit(2)
