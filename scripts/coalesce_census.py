#!/usr/bin/env python3
"""Measure how many register copies each coalescing rule can actually remove.

The backend peepholes eliminate a register copy (`mov xD, xS` / `mv rD, rS`)
by renaming `D` to `S` everywhere after it. The question every backend has to
answer is *when that is safe*, and each backend answers it differently:

  syntactic   the source must not be mentioned anywhere else in the function.
              Cheap and obviously safe, but blind: it rejects a copy whose
              source is used again later even when the value is already dead
              at the copy.
  liveness    the source must be dead immediately after the copy (backward
              dataflow), plus guards for the writes that would clobber the
              coalesced value. This is what the x86 backend does.

This tool counts, over the benchmark corpus, how many copies each rule admits
on the ASSEMBLER OUTPUT (i.e. after the peepholes have already run), so the
difference between the two columns is the work still on the table before any
compiler code is written. It is deliberately independent of the compiler: it
parses the emitted assembly, so it measures what shipped, not what the pass
believes it did.

Usage
-----
    python3 scripts/coalesce_census.py            # -O2 and -Os, both backends
    python3 scripts/coalesce_census.py -O3        # one opt level
    python3 scripts/coalesce_census.py --verbose  # per-program breakdown

Requires the `lccc-arm` / `lccc-riscv` binaries under target/fastbuild.
"""

import argparse
import glob
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PROG = os.path.join(ROOT, "tests", "benchmark", "programs")
BUILD = os.path.join(ROOT, "target", "fastbuild")

# The benchmark programs are written to be scaled by these macros; without
# them several default to sizes that are slow to compile or to run.
DEFS = (
    "-DBLOCK_COUNT=64 -DPASSES=1 -DXML_SIZE=4096 -DQUERY_COUNT=64 "
    "-DHAYSTACK_LEN=8192 -DWORD_COUNT=2048 -DDATA_SIZE=4096 "
    "-DVALUE_COUNT=4096 -DSRC_SIZE=4096 -DBUFFER_SIZE=4096 "
    "-DNODE_COUNT=512 -DLOOKUP_ROUNDS=1 -DTABLE_SIZE=512 -DNUM_OPS=256 "
    "-DSIZE=256 -DN=256 -DWIDTH=64 -DHEIGHT=64 -DNSTRINGS=32 "
    "-DMAX_LEN=32 -DLANES=4"
).split()

# Programs that need a toolchain feature this census does not set up.
SKIP = {"vecreg_new_ops.c", "tls_seg_access.c"}


# --------------------------------------------------------------- AArch64 ----
# x0-x18 are caller-saved; x0-x7 carry the return value.
ARM_CALLER_SAVED = frozenset(range(0, 19))
ARM_REGS = {}
for _i in range(31):
    ARM_REGS["x%d" % _i] = _i
    ARM_REGS["w%d" % _i] = _i          # a W access is an access to the family


def arm_parse(line):
    """(mnemonic, operands, defs, uses) for one AArch64 line, or None."""
    t = line.strip()
    if not t or t.startswith(".") or t.endswith(":"):
        return None
    head, _, rest = t.partition(" ")
    m = head
    ops = [o.strip() for o in rest.split(",")] if rest else []
    defs, uses = set(), set()

    def reg(tok):
        tok = tok.strip()
        if tok in ARM_REGS:
            return ARM_REGS[tok]
        return {"lr": 30, "sp": 31, "xzr": 31, "wzr": 31}.get(tok)

    if m == "ret":
        return m, ops, defs, set(range(0, 8))
    if m.startswith("bl"):
        return m, ops, ARM_CALLER_SAVED | {30}, set(range(0, 8))
    if m in ("b", "br") or re.match(r"^b\.\w+$", m):
        return m, ops, defs, uses
    if m in ("cbz", "cbnz", "tbz", "tbnz") and ops:
        r = reg(ops[0])
        if r is not None and r < 31:
            uses.add(r)
        return m, ops, defs, uses
    if m.startswith("st"):                       # store: everything is a read
        for o in ops:
            r = reg(o)
            if r is not None and r < 31:
                uses.add(r)
        return m, ops, defs, uses
    if m.startswith("ld"):                       # load: first operand written
        if ops:
            r = reg(ops[0])
            if r is not None and r < 31:
                defs.add(r)
            for o in ops[1:]:
                rr = reg(o)
                if rr is not None and rr < 31:
                    uses.add(rr)
        return m, ops, defs, uses
    for i, o in enumerate(ops):                  # default: destination first
        r = reg(o)
        if r is None or r >= 31:
            continue
        (defs if i == 0 else uses).add(r)
    return m, ops, defs, uses


def arm_copy(parsed):
    """(dst, src) for a plain 64-bit register-to-register move."""
    m, ops, _d, _u = parsed
    if m != "mov" or len(ops) != 2:
        return None
    d, s = ops[0].strip(), ops[1].strip()
    if d in ARM_REGS and s in ARM_REGS:
        return ARM_REGS[d], ARM_REGS[s]
    return None


# ---------------------------------------------------------------- RISC-V ----
# Register ids follow the peephole's numbering.
RV_NAMES = {"zero": 7, "ra": 40, "sp": 41, "gp": 8, "tp": 9}
for _k in range(0, 7):
    RV_NAMES["t%d" % _k] = _k
for _k in range(0, 12):
    RV_NAMES["s%d" % _k] = 10 + _k
for _k in range(0, 8):
    RV_NAMES["a%d" % _k] = 30 + _k
RV_UNTOUCHABLE = {7, 40, 41}                     # zero, ra, sp
RV_CALLER_SAVED = frozenset(list(range(30, 38)) + list(range(0, 7)))


def rv_parse(line):
    t = line.strip()
    if not t or t.startswith(".") or t.endswith(":"):
        return None
    head, _, rest = t.partition(" ")
    m = head
    ops = [o.strip() for o in rest.split(",")] if rest else []
    defs, uses = set(), set()

    def reg(tok):
        return RV_NAMES.get(tok.strip())

    def keep(r):
        return r is not None and r not in RV_UNTOUCHABLE

    if m == "ret":
        return m, ops, defs, set(range(30, 38))
    if m in ("ecall", "ebreak", "fence", "wfi"):
        return m, ops, defs, uses
    if m.startswith("jal") and m != "jal":
        return m, ops, RV_CALLER_SAVED | {40}, set(range(30, 38))
    if m.startswith("jump") or m.startswith("j "):
        return m, ops, defs, uses
    if re.match(r"^b(eq|ne|lt|ge|ltu|geu|eqz|nez|gz|lez|gez|lz)$", m):
        for o in ops[:2]:
            r = reg(o)
            if keep(r):
                uses.add(r)
        return m, ops, defs, uses
    if re.match(r"^s[wdhb]", m):                  # store
        for o in ops:
            r = reg(o)
            if keep(r):
                uses.add(r)
        return m, ops, defs, uses
    if m.startswith("l"):                         # load
        if ops:
            r = reg(ops[0])
            if keep(r):
                defs.add(r)
            for o in ops[1:]:
                rr = reg(o)
                if keep(rr):
                    uses.add(rr)
        return m, ops, defs, uses
    for i, o in enumerate(ops):
        r = reg(o)
        if not keep(r):
            continue
        (defs if i == 0 else uses).add(r)
    return m, ops, defs, uses


def rv_copy(parsed):
    m, ops, _d, _u = parsed
    if m == "mv" and len(ops) == 2:
        d, s = ops[0].strip(), ops[1].strip()
        if d in RV_NAMES and s in RV_NAMES:
            return RV_NAMES[d], RV_NAMES[s]
    if m == "addi" and len(ops) == 3 and ops[2].strip() == "0":
        d, s = ops[0].strip(), ops[1].strip()
        if d in RV_NAMES and s in RV_NAMES:
            return RV_NAMES[d], RV_NAMES[s]
    return None


# ------------------------------------------------------------- the census ----
def analyze(asm, parse, is_copy, nregs, caller_saved, is_branch, is_ret, is_call):
    """Return (syntactic_admitted, liveness_admitted) for one translation unit."""
    lines = [l.rstrip("\n") for l in asm]
    parsed = [parse(l) for l in lines]
    # Function boundaries: labels at column 0 that are not local (.L...).
    starts = [i for i, l in enumerate(lines)
              if l.endswith(":") and not l.startswith(".") and not l[:1].isspace()]
    total_syn = total_live = 0
    for si, st in enumerate(starts):
        end = starts[si + 1] if si + 1 < len(starts) else len(lines)
        # The entry run stops at the first label / branch / call / return.
        run_end = end
        for k in range(st + 1, end):
            l = lines[k].strip()
            if l.endswith(":") or is_branch(l) or is_call(l) or is_ret(l):
                run_end = k
                break
        blocks, bstart = [], st + 1
        for k in range(st + 1, end):
            l = lines[k].strip()
            if l.endswith(":") or is_branch(l) or is_ret(l):
                if k > bstart:
                    blocks.append((bstart, k))
                if is_branch(l) or is_ret(l):
                    blocks.append((k, k + 1))
                bstart = k + 1
        if bstart < end:
            blocks.append((bstart, end))
        if not blocks:
            continue
        labels = {}
        for bi, (a, _b) in enumerate(blocks):
            head = lines[a].strip()
            if head.endswith(":"):
                labels[head[:-1]] = bi
        succs = []
        for bi, (a, b) in enumerate(blocks):
            last = lines[b - 1].strip() if b > a else ""
            s = []
            if is_branch(last):
                tgt = last.split()[-1]
                if tgt in labels:
                    s.append(labels[tgt])
                if not (last.startswith("b ") or last.startswith("jump ")):
                    if bi + 1 < len(blocks):
                        s.append(bi + 1)
            elif not is_ret(last) and bi + 1 < len(blocks):
                s.append(bi + 1)
            succs.append(s)
        gen, kill = [], []
        for (a, b) in blocks:
            g, k_ = set(), set()
            for k in range(a, b):
                p = parsed[k]
                if p is None:
                    continue
                _m, _o, d, u = p
                g |= u - k_
                k_ |= d
            gen.append(g)
            kill.append(k_)
        live_out = [set() for _ in blocks]
        for _ in range(60):
            changed = False
            for bi in range(len(blocks) - 1, -1, -1):
                new = set()
                for s in succs[bi]:
                    new |= gen[s] | (live_out[s] - kill[s])
                if new != live_out[bi]:
                    live_out[bi] = new
                    changed = True
            if not changed:
                break
        for k in range(st + 1, run_end):
            p = parsed[k]
            if p is None:
                continue
            cp = is_copy(p)
            if cp is None:
                continue
            d, s = cp
            if d == s:
                continue
            bi = next((j for j, (a, b) in enumerate(blocks) if a <= k < b), None)
            if bi is None:
                continue
            a, b = blocks[bi]
            live = set(live_out[bi])
            for kk in range(b - 1, k, -1):
                pp = parsed[kk]
                if pp is None:
                    continue
                _m, _o, dd, uu = pp
                live = (live - dd) | uu
            src_dead_after = s not in live
            later_write = call_hazard = False
            for kk in range(k + 1, end):
                pp = parsed[kk]
                if pp is None:
                    continue
                _m, _o, dd, uu = pp
                if s in dd:
                    later_write = True
                    break
                if is_call(lines[kk].strip()) and s in caller_saved:
                    call_hazard = True
                    break
                if s in uu:
                    break                       # a read: rule 1 excludes it
            mentions_anywhere = any(
                (lambda pp: pp is not None and (s in pp[2] or s in pp[3]))(parsed[kk])
                for kk in range(st + 1, end) if kk != k
            )
            if not mentions_anywhere:
                total_syn += 1
            if src_dead_after and not later_write and not call_hazard:
                total_live += 1
    return total_syn, total_live


BACKENDS = [
    ("ARM", "lccc-arm", arm_parse, arm_copy, 31, ARM_CALLER_SAVED,
     lambda l: l.startswith("b.") or re.match(r"^b( |r)", l)
     or l.startswith("cb") or l.startswith("tb"),
     lambda l: l == "ret", lambda l: l.startswith("bl")),
    ("RISC-V", "lccc-riscv", rv_parse, rv_copy, 42, RV_CALLER_SAVED,
     lambda l: re.match(r"^(beq|bne|blt|bge|bltu|bgeu|beqz|bnez|j |jump )", l),
     lambda l: l == "ret", lambda l: l.startswith("jalr") or l.startswith("call")),
]


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--verbose", action="store_true",
                    help="report per program instead of per backend")
    # Optimisation levels look like flags to argparse, so they are collected
    # separately: `coalesce_census.py -O2 -Os`.
    args, opts = ap.parse_known_args()
    opts = opts or ["-O2", "-Os"]

    files = sorted(glob.glob(os.path.join(PROG, "*.c")))
    files = [f for f in files if os.path.basename(f) not in SKIP]
    if not files:
        print("no benchmark programs found under %s" % PROG, file=sys.stderr)
        return 1

    for name, binary, parse, is_copy, nregs, cs, br, rt, cl in BACKENDS:
        path = os.path.join(BUILD, binary)
        if not os.path.exists(path):
            print("%-7s: %s not built - run ./scripts/build_lccc_fast.sh" % (name, path))
            continue
        tot_s = tot_l = 0
        for f in files:
            s = l = 0
            for o in opts:
                try:
                    subprocess.run([path, o] + DEFS + [f, "-S", "-o", "/tmp/_census.s"],
                                   capture_output=True, timeout=120)
                except subprocess.TimeoutExpired:
                    continue
                if not os.path.exists("/tmp/_census.s"):
                    continue
                with open("/tmp/_census.s") as fh:
                    asm = fh.read().split("\n")
                os.remove("/tmp/_census.s")
                a, b = analyze(asm, parse, is_copy, nregs, cs, br, rt, cl)
                s += a
                l += b
            tot_s += s
            tot_l += l
            if args.verbose and (s or l):
                print("  %-28s syntactic %3d   liveness %3d" % (os.path.basename(f), s, l))
        print("%-7s %-11s syntactic(current) %4d   liveness(x86 rule) %4d   extra %4d"
              % (name, " ".join(opts), tot_s, tot_l, tot_l - tot_s))
    return 0


if __name__ == "__main__":
    sys.exit(main())
