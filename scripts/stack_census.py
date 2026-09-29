#!/usr/bin/env python3
"""Causal stack-reference census: WHY every stack reference exists (SPILL-01).

`ra_quality_census.py` answers "how many stack references does this function
emit".  The follow-up question — the one that decides where engineering time
goes — is "why is each of them there".  A slot holding a spilled register
candidate is an allocator problem; a slot holding an `alloca`, an
address-taken local, an i128/vector value or an outgoing-argument area is
structurally required memory that no allocator change can remove.  Tuning an
eviction policy against a number that is 90 % `alloca` traffic is how
optimization budgets are wasted.

Only the compiler knows the difference, so the compiler publishes it: with
`CCC_SLOT_CENSUS=1` lccc prints one `[SLOT-MAP]` line per function (offset,
cause, sharing count — see `src/backend/stack_layout/slot_census.rs`).  This
script joins that map with the POST-PEEPHOLE assembly and attributes every
emitted stack reference to a cause.  Measuring the assembly rather than the
internal layout is deliberate: peepholes delete accesses (dead stores, unused
callee saves) and add others (address materialization), so a census taken
before emission would report traffic that never reaches the object file.

Causes
------
    alloca   explicit alloca / parameter home: addressable by construction
    address  value has a register home AND a memory home (its address escapes)
    wide     i128 / vector / wide value: no single-register home exists
    spill    register pressure: an allocation candidate that lost
    nongpr   float / long-double the allocator cannot home in a GPR here
    temp     unclassified (reported separately; should shrink over time)
    csave    callee-saved register save/restore area (push/pop or the FPO slot)
    argout   outgoing call-argument area (below the frame)
    incoming stack-passed incoming argument (positive offset from %rbp)
    unknown  not covered by any rule — drives the coverage metric

Offset arithmetic
-----------------
`[SLOT-MAP]` offsets are absolute byte offsets from the ENTRY %rsp (negative).
The assembly uses %rsp/%rbp-relative displacements after the prologue, so the
script recovers the prologue geometry from the emitted text itself:

    p pushes, then `subq $N, %rsp`
        %rsp-based operand  ->  entry offset = disp - 8*p - N
        %rbp-based operand  ->  entry offset = disp - 8*p      (`movq %rsp,%rbp`)
        disp >= N (rsp)     ->  below the frame: outgoing argument area
        disp >= 8 (rbp)     ->  incoming stack argument

Usage:
    scripts/stack_census.py FILE.c ...          # census these files
    scripts/stack_census.py --corpus            # benchmark programs + kernels
    scripts/stack_census.py --json out.json     # machine-readable
    scripts/stack_census.py --min-coverage 95   # gate (default 95)
    scripts/stack_census.py --top 20            # 20 busiest functions

Environment:
    LCCC / CFLAGS / LCCC_FLAGS  as in ra_quality_census.py
"""
from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
DEFAULT_DIRS = [
    REPO / "tests" / "benchmark" / "programs",
    REPO / "tests" / "benchmark" / "kernel_corpus",
]
LCCC = os.environ.get("LCCC", str(REPO / "target" / "fastbuild" / "lccc"))
EXTRA = os.environ.get("CFLAGS", "").split()

CAUSES = (
    "alloca",
    "address",
    "wide",
    "spill",
    "nongpr",
    "temp",
    "csave",
    "argout",
    "incoming",
    "unknown",
)
# Causes a better allocator could remove.  Everything else is structural.
PRESSURE_CAUSES = ("spill",)

_SYMBOL = r"[A-Za-z_$][\w.$@]*"
_TYPE_FUNCTION = re.compile(rf"^\s*\.type\s+({_SYMBOL})\s*,\s*@function\b")
_SIZE_FUNCTION = re.compile(rf"^\s*\.size\s+({_SYMBOL})\s*,")
_DIRECTIVE = re.compile(r"^\s*\.")
_COMMENT = re.compile(r"^\s*#")
_PUSH = re.compile(r"^\s*push[qwl]?\s+%")
_POP = re.compile(r"^\s*pop[qwl]?\s+%")
_SUB_RSP = re.compile(r"^\s*sub[qwl]?\s+\$(\d+),\s*%rsp\b")
_ADD_RSP = re.compile(r"^\s*add[qwl]?\s+\$(\d+),\s*%rsp\b")
_MOV_RBP = re.compile(r"^\s*mov[qwl]?\s+%rsp,\s*%rbp\b")
# A memory operand with an %rsp/%rbp base: `16(%rsp)`, `-8(%rbp)`, `(%rsp)`,
# `16(%rsp,%rdi)`, `(%rsp,%rax,4)`.  The base must be inside the parentheses.
_MEM = re.compile(r"(?P<disp>[+-]?\d*)\(\s*%(?P<base>rsp|rbp)\b(?P<rest>[^)]*)\)")
_LEA = re.compile(r"^\s*lea[qwl]?\b")
_SLOT_MAP = re.compile(
    r"^\[SLOT-MAP\] fn=(?P<fn>\S+) frame=(?P<frame>-?\d+) base=(?P<base>\S+) "
    r"csave=(?P<csave>\d+) slots=(?P<slots>.*)$"
)
_RA_STATS = re.compile(
    r"^\[RA-STATS\] fn=(?P<fn>\S+) " + r".*?spilled=(?P<spilled>\d+)"
)


class Function:
    """One function's census."""

    def __init__(self, name: str) -> None:
        self.name = name
        self.frame = 0
        self.base = "rsp"
        self.csave = 0
        self.slots: dict[int, tuple[str, int]] = {}
        self.insns = 0
        # cause -> {"load": n, "store": n, "addr": n}
        self.traffic: dict[str, dict[str, int]] = {}
        self.spilled_ra: int | None = None
        self.unmatched: list[tuple[str, int]] = []
        self.indexed = 0
        # slot offset -> {"load": n, "store": n, "addr": n}: RA-GLA-04's
        # post-RA traffic feedback, per planned location piece.
        self.slot_traffic: dict[int, dict[str, int]] = {}

    def add(self, cause: str, kind: str) -> None:
        bucket = self.traffic.setdefault(cause, {"load": 0, "store": 0, "addr": 0})
        bucket[kind] += 1

    @property
    def refs(self) -> int:
        return sum(sum(b.values()) for b in self.traffic.values())

    def by_cause(self) -> dict[str, int]:
        return {c: sum(self.traffic.get(c, {}).values()) for c in CAUSES}

    def covered(self) -> int:
        return self.refs - sum(self.traffic.get("unknown", {}).values())


def parse_slot_map(stderr: str) -> dict[str, Function]:
    funcs: dict[str, Function] = {}
    for line in stderr.splitlines():
        m = _SLOT_MAP.match(line.strip())
        if not m:
            continue
        fn = Function(m.group("fn"))
        fn.frame = int(m.group("frame"))
        fn.base = m.group("base")
        fn.csave = int(m.group("csave"))
        for item in m.group("slots").split("|"):
            if not item:
                continue
            parts = item.split(":")
            if len(parts) != 3:
                continue
            fn.slots[int(parts[0])] = (parts[1], int(parts[2]))
        # A repeated function keeps the LAST layout (the prologue refines it).
        funcs[fn.name] = fn
        ra = _RA_STATS.match(line.strip())
        if ra:
            fn.spilled_ra = int(ra.group("spilled"))
    for line in stderr.splitlines():
        ra = _RA_STATS.match(line.strip())
        if ra and ra.group("fn") in funcs:
            funcs[ra.group("fn")].spilled_ra = int(ra.group("spilled"))
    return funcs


def classify_operand(fn: Function, disp: int, base: str, npush: int,
                     frame_size: int, is_lea: bool,
                     indexed: bool = False) -> tuple[str, str, int, int | None]:
    """Map one memory operand to (cause, access kind, entry-frame offset).

    `indexed` operands carry a dynamic index register (`16(%rsp,%rdx)`), so
    their displacement is only the base of a walked range: the census
    attributes them to the nearest declared slot instead of demanding an exact
    hit, because "which element of m" is unknowable statically while "which
    object" is exactly the question being asked.
    """
    """Map one memory operand to (cause, access kind)."""
    kind = "addr" if is_lea else "load"
    if base == "rsp":
        if disp >= frame_size:
            return "argout", kind, disp, None
        off = disp - 8 * npush - frame_size
    else:  # %rbp
        if disp >= 8:
            return "incoming", kind, disp, None
        off = disp - 8 * npush
    if -8 * fn.csave < off <= 0 and fn.csave > 0:
        return "csave", kind, off, None
    slot_offset = off if off in fn.slots else None
    slot = fn.slots.get(off)
    if slot is None:
        # A reference into a slot's interior (m[4] of an array alloca) still
        # belongs to that slot: walk back to the nearest declared slot at or
        # below the access.  The walk must stay inside that slot, so the
        # candidate has to be within 4 KiB — otherwise the reference really is
        # outside the declared frame (an unattributed access, not an element).
        below = [o for o in fn.slots if o <= off and off - o < 4096]
        if below:
            slot_offset = max(below)
            slot = fn.slots[slot_offset]
        elif indexed and fn.slots:
            # Base before the first declared slot (the index walks forward into
            # the object): take the nearest slot in either direction, provided
            # it is inside this frame.
            nearest = min(fn.slots, key=lambda o: abs(o - off))
            if abs(nearest - off) <= max(fn.frame, frame_size):
                slot_offset = nearest
                slot = fn.slots[nearest]
    if slot is None:
        return "unknown", kind, off, None
    return slot[0], kind, off, slot_offset


def analyze_asm(path: Path, funcs: dict[str, Function]) -> None:
    lines = path.read_text().splitlines()
    current: Function | None = None
    npush = 0
    frame_size = 0
    seen_sub = False
    for raw in lines:
        m = _TYPE_FUNCTION.match(raw)
        if m:
            current = funcs.get(m.group(1))
            npush = 0
            frame_size = 0
            seen_sub = False
            continue
        if current is None:
            continue
        if _SIZE_FUNCTION.match(raw):
            current = None
            continue
        if not raw.strip() or _COMMENT.match(raw) or _DIRECTIVE.match(raw):
            continue
        current.insns += 1
        if _PUSH.match(raw):
            npush += 1
            current.add("csave", "store")
            continue
        if _POP.match(raw):
            current.add("csave", "load")
            continue
        if _MOV_RBP.match(raw):
            continue
        if not seen_sub:
            sub = _SUB_RSP.match(raw)
            if sub:
                frame_size = int(sub.group(1))
                seen_sub = True
                continue
        # Skip the epilogue's frame release: it is not slot traffic.
        if _ADD_RSP.match(raw) and seen_sub and raw.strip().startswith("addq $"):
            if frame_size and int(_ADD_RSP.match(raw).group(1)) == frame_size:
                continue
        is_lea = bool(_LEA.match(raw))
        store = False
        if not is_lea:
            # `movX %reg, disp(%rsp)` and friends: operand order decides.
            comma = raw.find(",")
            head, tail = (raw[:comma], raw[comma + 1:]) if comma > 0 else (raw, "")
            is_mem_dest = bool(_MEM.search(tail)) and not _MEM.search(head)
            is_mem_src = bool(_MEM.search(head))
            store = is_mem_dest and not is_mem_src
        for m in _MEM.finditer(raw):
            disp_text = m.group("disp")
            if disp_text in ("", "+"):
                disp = 0
            elif disp_text == "-":
                disp = 0
            else:
                disp = int(disp_text)
            base = m.group("base")
            # An index register means the displacement is only the base of a
            # walked range (m[i]); the census attributes it to that base slot.
            indexed = "," in m.group("rest")
            cause, kind, off, slot_off = classify_operand(
                current, disp, base, npush, frame_size, is_lea, indexed)
            if indexed:
                current.indexed += 1
            if store and not is_lea:
                kind = "store"
            current.add(cause, kind)
            if slot_off is not None:
                bucket = current.slot_traffic.setdefault(
                    slot_off, {"load": 0, "store": 0, "addr": 0})
                bucket[kind] += 1
            if cause == "unknown":
                current.unmatched.append((raw.strip(), off))


def census_file(cc: str, src: Path, flags: list[str]) -> dict[str, Function]:
    with tempfile.TemporaryDirectory() as tmp:
        asm = Path(tmp) / "out.s"
        env = dict(os.environ)
        env["CCC_SLOT_CENSUS"] = "1"
        proc = subprocess.run(
            [cc, *flags, "-S", "-o", str(asm), str(src)],
            capture_output=True, text=True, env=env,
        )
        if proc.returncode != 0:
            print(f"  skip {src.name}: compile failed", file=sys.stderr)
            return {}
        funcs = parse_slot_map(proc.stderr)
        if not funcs:
            return {}
        analyze_asm(asm, funcs)
        return funcs


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("files", nargs="*", type=Path)
    ap.add_argument("--corpus", action="store_true",
                    help="benchmark programs + kernel corpus")
    ap.add_argument("--json", type=Path)
    ap.add_argument("--top", type=int, default=0)
    ap.add_argument("--min-coverage", type=float, default=95.0)
    ap.add_argument("--per-slot", type=int, default=0, metavar="N",
                    help="list the N busiest slots (RA-GLA-04 traffic feedback)")
    ap.add_argument("--show-unknown", action="store_true",
                    help="list every unattributed reference (drives the gap down)")
    ap.add_argument("--lccc", default=LCCC)
    ap.add_argument("--cflags", default=" ".join(EXTRA))
    args = ap.parse_args(argv[1:])

    files = list(args.files)
    if args.corpus or not files:
        for d in DEFAULT_DIRS:
            if d.is_dir():
                files += sorted(p for p in d.glob("*.c"))
    if not files:
        print("stack_census: no input files", file=sys.stderr)
        return 2

    flags = ["-O2", *args.cflags.split()] if args.cflags else ["-O2"]
    totals: dict[str, int] = {c: 0 for c in CAUSES}
    per_func: list[Function] = []
    per_file: dict[str, dict[str, int]] = {}
    for src in files:
        funcs = census_file(args.lccc, src, flags)
        if not funcs:
            continue
        agg: dict[str, int] = {c: 0 for c in CAUSES}
        for fn in funcs.values():
            per_func.append(fn)
            for cause, n in fn.by_cause().items():
                agg[cause] += n
                totals[cause] += n
        agg["insns"] = sum(f.insns for f in funcs.values())
        per_file[src.name] = agg

    total_refs = sum(totals.values())
    covered = total_refs - totals["unknown"]
    coverage = 100.0 * covered / total_refs if total_refs else 100.0

    print("── per-file stack-reference census "
          f"({' '.join(flags)})")
    print(f"   {'file':<34s} {'insns':>7s} {'refs':>6s} " +
          "".join(f"{c[:6]:>7s}" for c in CAUSES))
    for name, agg in sorted(per_file.items(),
                            key=lambda kv: -sum(
                                v for k, v in kv[1].items() if k != "insns")):
        if sum(v for k, v in agg.items() if k != "insns") == 0:
            continue
        print(f"   {name:<34s} {agg['insns']:7d} "
              f"{sum(v for k, v in agg.items() if k != 'insns'):6d} " +
              "".join(f"{agg[c]:7d}" for c in CAUSES))
    print("   " + "─" * (34 + 7 + 7 + 7 * len(CAUSES)))
    print(f"   {'TOTAL':<34s} {'':7s} {total_refs:6d} " +
          "".join(f"{totals[c]:7d}" for c in CAUSES))
    print()
    pressure = sum(totals[c] for c in PRESSURE_CAUSES)
    print(f"   coverage            : {coverage:.2f}% "
          f"({covered}/{total_refs} references attributed)")
    print(f"   pressure-driven     : {pressure} refs "
          f"({100.0 * pressure / total_refs if total_refs else 0:.1f}%)")
    structural = sum(totals[c] for c in ("alloca", "address", "wide", "nongpr"))
    print(f"   structural (memory) : {structural} refs "
          f"({100.0 * structural / total_refs if total_refs else 0:.1f}%)")
    print(f"   frame protocol      : {totals['csave']} saves, "
          f"{totals['argout']} outgoing-arg, {totals['incoming']} incoming-arg")

    if args.per_slot:
        print()
        print(f"── top {args.per_slot} slots by post-RA traffic "
              "(offset from entry %rsp)")
        print(f"   {'function':<28s} {'offset':>8s} {'cause':>8s} "
              f"{'load':>6s} {'store':>6s} {'addr':>6s} {'total':>6s}")
        rows = []
        for fn in per_func:
            for off, bucket in fn.slot_traffic.items():
                rows.append((sum(bucket.values()), fn, off, bucket))
        rows.sort(key=lambda r: -r[0])
        for total, fn, off, bucket in rows[: args.per_slot]:
            cause = fn.slots.get(off, ("?", 0))[0]
            print(f"   {fn.name[:28]:<28s} {off:8d} {cause:>8s} "
                  f"{bucket['load']:6d} {bucket['store']:6d} "
                  f"{bucket['addr']:6d} {total:6d}")
        if not rows:
            print("   (none)")

    if args.show_unknown:
        print()
        print("── unattributed references")
        shown = 0
        for fn in per_func:
            for text, off in fn.unmatched:
                print(f"   {fn.name}: entry-offset {off:>6d}  {text}")
                shown += 1
        if shown == 0:
            print("   (none)")

    if args.top:
        print()
        print(f"── top {args.top} functions by stack references")
        print(f"   {'function':<40s} {'refs':>6s} {'spill':>6s} "
              f"{'alloca':>6s} {'insns':>6s}")
        for fn in sorted(per_func, key=lambda f: -f.refs)[: args.top]:
            bc = fn.by_cause()
            print(f"   {fn.name[:40]:<40s} {fn.refs:6d} {bc['spill']:6d} "
                  f"{bc['alloca']:6d} {fn.insns:6d}")

    if args.json:
        payload = {
            "flags": flags,
            "totals": totals,
            "coverage_pct": coverage,
            "files": per_file,
            "functions": [
                {
                    "name": f.name,
                    "insns": f.insns,
                    "frame": f.frame,
                    "base": f.base,
                    "csave": f.csave,
                    "refs": f.refs,
                    "indexed_refs": f.indexed,
                    "by_cause": f.by_cause(),
                    "traffic": f.traffic,
                    "slot_traffic": {str(o): b for o, b in f.slot_traffic.items()},
                    "slots": {str(o): c for o, c in f.slots.items()},
                }
                for f in per_func
            ],
        }
        args.json.write_text(json.dumps(payload, indent=2) + "\n")
        print(f"\n   wrote {args.json}")

    if coverage < args.min_coverage:
        print(f"\nstack_census: FAIL coverage {coverage:.2f}% "
              f"< {args.min_coverage:.2f}% (gate)", file=sys.stderr)
        return 1
    print("\nstack_census: PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
