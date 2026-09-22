#!/usr/bin/env python3
"""boot_insn_census.py — where do the extra i686 boot-code bytes come from?

`boot_size_oracle.sh` answers *how much* larger LCCC's `arch/x86/boot`
objects are than a reference compiler's (currently ~+10.7 KiB over 23
objects, i.e. the 32 KiB setup gate runs at 1.6 KiB of headroom instead of
~9.9 KiB).  It does not answer *what kind of instruction* the surplus is
made of, which is the only question that tells you which pass to change.

This script compiles every boot translation unit with both toolchains using
the shared command lines from `scripts/boot_flags.sh` (so a delta is a
codegen delta and nothing else), strips the assembly of directives and
pseudo-ops, and reports:

  * per-mnemonic counts for each toolchain, sorted by |delta|;
  * the size-class breakdown of the surplus (how many bytes each family
    could plausibly account for, using the i386 encoding lengths the
    emitter actually produces);
  * a set of structural counters that name whole *defect classes* rather
    than mnemonics: prologue pushes, frame size, parameter homing stores,
    reload-of-just-spilled, redundant sign/zero extension, `jmp` to the
    shared epilogue, and stack-slot references in compares.

The structural counters are the point: "lccc emits 412 more `movl`" is not
actionable, "lccc spends 187 instructions storing register-passed
parameters into frame slots it then reloads" names a pass.

Usage:
    scripts/boot_insn_census.py                     # gcc oracle
    scripts/boot_insn_census.py --oracle clang
    scripts/boot_insn_census.py --top 40 --show-classes
"""
from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
from collections import Counter, defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent
DEFAULT_KERNEL = os.environ.get("KERNEL_DIR", "/home/user/kernel-work/linux-6.18.52")

# Sourced from boot_flags.sh rather than duplicated: the whole value of this
# tool is that both arms see the identical command line.
def boot_flags(kernel: Path) -> tuple[str, str]:
    sh = (
        f'. {HERE}/boot_flags.sh; '
        'printf "%s\\n" "$LCCC_BOOT_CFLAGS"; printf "%s\\n" "$LCCC_BOOT_CPPFLAGS"'
    )
    out = subprocess.run(
        ["bash", "-c", sh], cwd=kernel, capture_output=True, text=True, check=True
    ).stdout.splitlines()
    return out[0], out[1]


def cflags_for(oracle: str, base: str) -> str:
    """Mirror boot_flags.sh's lccc_boot_cflags_for for the oracle side."""
    if "clang" in os.path.basename(oracle):
        return base.replace("-mpreferred-stack-boundary=2", "-mstack-alignment=4") + " -Wno-gnu"
    return base


DIRECTIVE = re.compile(r"^\s*(\.|#|/\*|--|@)")
LABEL = re.compile(r"^\s*[.\w$][\w$.]*\s*:")
INSN = re.compile(r"^\s*([a-z][a-z0-9]*)\b(.*)$")


def parse_asm(path: Path) -> list[tuple[str, str]]:
    """Return [(mnemonic, operand-text)] for real instructions only."""
    insns: list[tuple[str, str]] = []
    in_app = False
    for raw in path.read_text(errors="replace").splitlines():
        line = raw.split("//")[0]
        s = line.strip()
        if s == "#APP":
            in_app = True
            continue
        if s == "#NO_APP":
            in_app = False
            continue
        if in_app:  # inline asm: belongs to the source, not to the backend
            continue
        if not s or DIRECTIVE.match(line) or LABEL.match(line):
            continue
        m = INSN.match(line)
        if not m:
            continue
        insns.append((m.group(1), m.group(2).strip()))
    return insns


def frame_sizes(insns: list[tuple[str, str]]) -> list[int]:
    """Every `subl $N,%esp` that opens a frame (heuristic: any esp adjust)."""
    out = []
    for mn, ops in insns:
        if mn in ("subl", "subw") and "%esp" in ops and "$" in ops:
            m = re.search(r"\$(-?\d+)", ops)
            if m:
                out.append(int(m.group(1)))
    return out


def structural(insns: list[tuple[str, str]]) -> dict[str, int]:
    """Defect-class counters. Each one names a pass, not a mnemonic."""
    c: dict[str, int] = defaultdict(int)
    prev: list[tuple[str, str]] = []
    for i, (mn, ops) in enumerate(insns):
        if mn in ("pushl", "pushw") and re.search(r"%e?bx|%e?si|%e?di|%e?bp", ops):
            c["prologue-callee-save-push"] += 1
        # `movl %reg, N(%esp)` — storing into a frame slot.
        if mn == "movl" and re.search(r",\s*-?\d+\(%esp\)\s*$", ops):
            c["store-to-frame-slot"] += 1
            # Reload of a value that was stored to the same slot within the
            # previous 6 instructions: the store existed only to be read back.
            slot = re.search(r"(-?\d+)\(%esp\)", ops)
            if slot:
                for pmn, pops in insns[max(0, i - 6) : i]:
                    if (
                        pmn == "movl"
                        and f"{slot.group(1)}(%esp)" in pops
                        and pops.strip().startswith(f"{slot.group(1)}(%esp)")
                    ):
                        c["store-then-reload-same-slot"] += 1
                        break
        if mn == "movl" and re.search(r"-?\d+\(%esp\),\s*%e", ops):
            c["load-from-frame-slot"] += 1
        # Redundant narrow widening right after a narrowing load/move.
        if mn in ("movsbl", "movzbl") and re.match(r"^%[a-d]l,", ops):
            c["reg-to-reg-narrow-extend"] += 1
        if mn == "movzwl" and re.match(r"^%[a-d]x,", ops):
            c["reg-to-reg-word-extend"] += 1
        # Compare that reads memory instead of a register.
        if mn in ("cmpl", "cmpw", "cmpb", "testl") and re.search(r"-?\d+\(%(esp|ebp)\)", ops):
            c["compare-against-frame-slot"] += 1
        # Register-to-register move: pure coalescing failure.
        if mn == "movl" and re.match(r"^%e[a-dsbd][xhlbp]*,\s*%e", ops):
            c["reg-to-reg-movl"] += 1
        if mn == "jmp" and "_epilogue_" in ops:
            c["jmp-to-shared-epilogue"] += 1
        if mn == "call" and "get_pc_thunk" in ops:
            c["pic-pc-thunk"] += 1
        # Instruction that only exists to widen/narrow the accumulator.
        if mn == "leal" and re.search(r"\(%e[a-d]x,%e[a-d]x", ops):
            c["lea-scaled-self"] += 1
        prev.append((mn, ops))
    frames = frame_sizes(insns)
    c["fn-count"] = sum(1 for mn, _ in insns if mn == "ret" or mn == "retw")
    c["total-frame-bytes"] = sum(frames)
    c["functions-with-frame"] = len(frames)
    return c


# Rough i386 encoding costs for the families that dominate the surplus, so a
# mnemonic delta can be read as a byte delta.  These are the lengths the
# emitter produces for the boot code's shapes (32-bit operand, ModRM+disp8/
# disp32, SIB where indexed) — deliberately conservative.
BYTES = {
    "movl": 3.5,
    "movzbl": 3.5,
    "movsbl": 3.5,
    "movzwl": 3.5,
    "pushl": 1.5,
    "popl": 1.5,
    "subl": 3.0,
    "addl": 3.0,
    "cmpl": 4.0,
    "jmp": 2.5,
    "je": 2.0,
    "jne": 2.0,
}


def compile_all(cc: str, cflags: str, cppflags: str, kernel: Path, out: Path) -> dict[str, Path]:
    out.mkdir(parents=True, exist_ok=True)
    res: dict[str, Path] = {}
    src = sorted((kernel / "arch/x86/boot").glob("*.c"))
    for s in src:
        dst = out / f"{s.stem}.s"
        cmd = [cc, *cflags.split(), *cppflags.split(), "-DSVGA_MODE=NORMAL_VGA", "-S", str(s), "-o", str(dst)]
        p = subprocess.run(cmd, cwd=kernel, capture_output=True, text=True)
        if p.returncode != 0 or not dst.exists():
            print(f"  skip {s.name}: {cc} failed", file=sys.stderr)
            continue
        res[s.stem] = dst
    return res


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--kernel", default=DEFAULT_KERNEL)
    ap.add_argument("--oracle", default="gcc")
    ap.add_argument("--lccc", default=str(REPO / "target/fastbuild/lccc"))
    ap.add_argument("--top", type=int, default=30)
    ap.add_argument("--work", default="/tmp/boot-census")
    ap.add_argument("--show-classes", action="store_true")
    a = ap.parse_args()

    kernel = Path(a.kernel)
    if not kernel.is_dir():
        print(f"kernel tree missing: {kernel}", file=sys.stderr)
        return 2
    cflags, cppflags = boot_flags(kernel)

    lccc_asm = compile_all(a.lccc, cflags, cppflags, kernel, Path(a.work) / "lccc")
    orc_asm = compile_all(a.oracle, cflags_for(a.oracle, cflags), cppflags, kernel, Path(a.work) / a.oracle)
    common = sorted(set(lccc_asm) & set(orc_asm))
    if not common:
        print("no translation unit compiled with both toolchains", file=sys.stderr)
        return 2

    per_class: dict[str, dict[str, int]] = {}
    mn_total: dict[str, Counter] = {"lccc": Counter(), "oracle": Counter()}
    for name in common:
        li = parse_asm(lccc_asm[name])
        oi = parse_asm(orc_asm[name])
        per_class[name] = {
            "lccc": len(li),
            "oracle": len(oi),
        }
        mn_total["lccc"].update(m for m, _ in li)
        mn_total["oracle"].update(m for m, _ in oi)

    print(f"corpus: {len(common)} translation units, {a.kernel}")
    print(f"lccc: {a.lccc}\noracle: {a.oracle} ({cflags_for(a.oracle, cflags)[:60]}...)")
    tl = sum(v["lccc"] for v in per_class.values())
    to = sum(v["oracle"] for v in per_class.values())
    print(f"\ntotal instructions: lccc={tl}  {a.oracle}={to}  delta={tl - to:+d} ({100.0 * (tl - to) / max(to, 1):+.1f}%)")

    print(f"\n== worst translation units by instruction delta ==")
    print(f"{'TU':<24}{'lccc':>8}{a.oracle:>8}{'delta':>8}")
    for name, v in sorted(per_class.items(), key=lambda kv: kv[1]["oracle"] - kv[1]["lccc"])[:12]:
        print(f"{name:<24}{v['lccc']:>8}{v['oracle']:>8}{v['lccc'] - v['oracle']:>+8}")

    print(f"\n== mnemonic deltas (top {a.top} by |delta|) ==")
    print(f"{'mnemonic':<16}{'lccc':>8}{a.oracle:>8}{'delta':>8}{'~bytes':>9}")
    deltas = {m: mn_total["lccc"][m] - mn_total["oracle"][m] for m in set(mn_total["lccc"]) | set(mn_total["oracle"])}
    est = 0.0
    for m, d in sorted(deltas.items(), key=lambda kv: -abs(kv[1]))[: a.top]:
        b = d * BYTES.get(m, 3.0)
        est += b
        print(f"{m:<16}{mn_total['lccc'][m]:>8}{mn_total['oracle'][m]:>8}{d:>+8}{b:>+9.0f}")
    print(f"{'(top-N estimate)':<16}{'':>24}{est:>+9.0f} bytes")

    # Structural counters, summed over the corpus.
    sl: Counter = Counter()
    so: Counter = Counter()
    for name in common:
        sl.update(structural(parse_asm(lccc_asm[name])))
        so.update(structural(parse_asm(orc_asm[name])))
    print("\n== structural defect-class counters ==")
    print(f"{'class':<36}{'lccc':>8}{a.oracle:>8}{'delta':>8}")
    for k in sorted(set(sl) | set(so), key=lambda k: -(sl[k] - so[k])):
        if k in ("fn-count",):
            continue
        print(f"{k:<36}{sl[k]:>8}{so[k]:>8}{sl[k] - so[k]:>+8}")
    print(f"{'functions (ret count)':<36}{sl['fn-count']:>8}{so['fn-count']:>8}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
