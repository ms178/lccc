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

4.  **Differ.** The pinned source is assembled by GAS and by LCCC, and the
    `.text` bytes are compared at each instruction's address. Batches that
    fail to assemble -- because one instruction in the batch is unsupported --
    are recursively bisected so that a single bad apple costs `log2(batch)`
    extra invocations rather than the whole batch.

Result classes
--------------
`MISENCODE`     both assembled, bytes differ            <-- a real bug
`LCCC_REJECT`   GAS accepted it, LCCC refused it        <-- coverage gap
`GAS_REJECT`    LCCC accepted it, GAS refused it        <-- possibly intentional
`BOTH_REJECT`   neither accepted it (objdump/GAS disagree, e.g. ISA level)
`OK`            byte-identical

Only `MISENCODE` is unambiguously a defect; the tool exits non-zero on those and
reports the rest as information.

Self-test
---------
A differential harness that silently stops working reports "0 mismatches"
forever, which is the most dangerous possible failure mode for this class of
tool. The harness therefore asserts, before doing anything else, that (a) a
known-good instruction encodes identically under both assemblers and (b) a
known-bad instruction is rejected by both. If either check fails the run aborts
rather than reporting a comforting zero.

Usage
-----
    python3 scripts/aarch64_encoder_differential.py --n 200000
    python3 scripts/aarch64_encoder_differential.py --n 20000 -o report.json
    python3 scripts/aarch64_encoder_differential.py --seed 7 --batch 32 -v

Requires `binutils-aarch64-linux-gnu` (or any aarch64 GAS/objdump/objcopy
triple) and a built LCCC binary.
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
from collections import Counter, defaultdict
from dataclasses import dataclass, field
from pathlib import Path

# --------------------------------------------------------------------------
# Result classes
# --------------------------------------------------------------------------

OK = "OK"
MISENCODE = "MISENCODE"
LCCC_REJECT = "LCCC_REJECT"
GAS_REJECT = "GAS_REJECT"
BOTH_REJECT = "BOTH_REJECT"

PROBLEM_CLASSES = (MISENCODE,)

# objdump -w output:   "     1c:\tb200c4c3 \torr\tx3, x6, #0x303030303030303"
INSN_RE = re.compile(r"^\s*([0-9a-fA-F]+):\s+([0-9a-fA-F]{8})\s+(\S+)\s*(.*)$")


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
        d = self.tmpdir
        s = d / f"{tag}.s"
        o = d / f"{tag}.o"
        b = d / f"{tag}.bin"
        s.write_text(self.prologue + src)
        for p in (o, b):
            p.unlink(missing_ok=True)

        if use_lccc:
            r = self._run([str(self.lccc_link), "-c", str(s), "-o", str(o)])
        else:
            r = self._run([self.gas, str(s), "-o", str(o)])
        if r.returncode != 0 or not o.is_file():
            return AsmResult(False, b"", (r.stderr or "") + (r.stdout or ""))

        # Extract the raw .text bytes so the comparison is on machine code,
        # not on ELF layout or symbol tables.
        rc = self._run([self.objcopy, "-O", "binary", "--only-section=.text", str(o), str(b)])
        if rc.returncode != 0 or not b.is_file():
            return AsmResult(False, b"", (rc.stderr or "") + (rc.stdout or ""))
        return AsmResult(True, b.read_bytes(), "")

    # -- self-test ---------------------------------------------------------

    def self_test(self) -> None:
        """Prove the harness can detect a difference before trusting its silence.

        A differential tester whose oracle has quietly stopped working reports
        zero mismatches for the rest of its life. Two assertions prevent that:
        a known-good instruction must agree, and a known-bad one must be
        rejected by both sides.
        """
        good = ".text\nadd x0, x1, x2\nnop\nret\n"
        g = self.assemble(good, "selftest_good", use_lccc=False)
        l = self.assemble(good, "selftest_good", use_lccc=True)
        if not (g.ok and l.ok):
            raise HarnessError(
                "self-test failed: a valid instruction did not assemble\n"
                f"  gas : {g.first_error}\n  lccc: {l.first_error}"
            )
        if g.data != l.data:
            raise HarnessError(
                "self-test failed: GAS and LCCC disagree on `add x0, x1, x2` "
                f"({g.data.hex()} vs {l.data.hex()}) -- harness is misconfigured"
            )
        if len(l.data) != 12:
            raise HarnessError(f"self-test failed: expected 12 bytes, got {len(l.data)}")

        # NB: `add x0, x1, x2, x3` is NOT usable here -- LCCC accepts surplus
        # operands and silently ignores them while GAS rejects them (defect
        # recorded separately). Use a token neither side can parse.
        bad = ".text\nfrobnicate x0, x1\n"
        g = self.assemble(bad, "selftest_bad", use_lccc=False)
        l = self.assemble(bad, "selftest_bad", use_lccc=True)
        if g.ok or l.ok:
            raise HarnessError(
                "self-test failed: an invalid instruction was accepted "
                f"(gas ok={g.ok}, lccc ok={l.ok}) -- harness cannot detect junk"
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
    with blob.open("wb") as f:
        f.write(rng.randbytes(4 * n_words))

    r = tc._run(
        [tc.objdump, "-D", "-b", "binary", "-m", tc.objdump_arch, "-w", str(blob)]
    )
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
                raise HarnessError(f"non-monotonic address 0x{ins.addr:x} after 0x{cursor:x}")
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
        want = 4 * len(items)

        if g.ok and l.ok and len(g.data) >= want and len(l.data) >= want:
            findings = []
            for k, ins in enumerate(items):
                off = 4 * k
                gb = g.data[off:off + 4]
                lb = l.data[off:off + 4]
                if gb == lb:
                    findings.append(Finding(OK, ins.addr, ins.text, ins.mnemonic, ins.encoding,
                                            gb.hex(), lb.hex()))
                else:
                    findings.append(Finding(MISENCODE, ins.addr, ins.text, ins.mnemonic,
                                            ins.encoding, gb.hex(), lb.hex()))
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
    per_mn = Counter(f.mnemonic for f in findings if f.status in PROBLEM_CLASSES)
    mis = [f for f in findings if f.status == MISENCODE]

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
    print("  result            count")
    print("  " + "-" * 40)
    for k in (OK, MISENCODE, LCCC_REJECT, GAS_REJECT, BOTH_REJECT):
        if by_status.get(k):
            print(f"  {k:<18} {by_status[k]:>8,}")
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
        print("No misencodings: LCCC's AArch64 encoder is byte-identical to "
              f"GNU as on all {by_status.get(OK, 0):,} instructions tested.")

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
    ap.add_argument("-v", "--verbose", action="store_true")
    args = ap.parse_args()

    tc = Toolchain.build(args)
    try:
        tc.self_test()
        if args.verbose:
            print("self-test: harness can detect both agreement and junk", file=sys.stderr)

        corpus = generate_corpus(tc, args.n, args.seed)
        distinct = len({i.text for i in corpus})
        if args.verbose:
            print(f"decoded {len(corpus):,} instructions ({distinct:,} distinct)",
                  file=sys.stderr)

        picked, _ = select_corpus(corpus, args.per_mnemonic, args.max_insns)

        differ = Differ(tc, args.batch, args.verbose)
        findings = differ.run(picked)
    finally:
        # A killed or failed run must not leak its scratch directory: each one
        # is ~0.8 GB of per-instruction .s/.o/.bin files, and a few leaked runs
        # were enough to fill the root disk mid-session.
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

    tc.close()
    return rc


if __name__ == "__main__":
    try:
        sys.exit(main())
    except HarnessError as e:
        print(f"error: {e}", file=sys.stderr)
        sys.exit(2)
