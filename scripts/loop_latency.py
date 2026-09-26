#!/usr/bin/env python3
"""Recurrence latency bound of a loop in AT&T x86 assembly.

For a latency-bound loop (a serial dependence chain carried around the
backedge, e.g. an ARX cipher's round loop) the steady-state time per
iteration is bounded below by its critical recurrence, independent of
issue width or port count.  This tool computes exactly that bound with
unlimited execution resources: it replays the loop body many times,
propagating per-register ready times across iterations, and reports the
steady-state growth of the ready time per iteration.

It is deliberately *not* a throughput model (use llvm-mca for that); it is
the one number an instruction-count or throughput view cannot see, and the
one the vec_arx lane-frame choice optimises.  The latency table covers the
integer SIMD/GPR ops those loops use; an unknown mnemonic is a hard error,
so a new instruction can never silently count as free.

Latencies are 1 cycle for every simple vector/GPR integer op (add, xor, or,
and, shifts, pshufb, pshufd, vprold, rol/ror/rorx) and 0 for reg-reg moves
(eliminated at rename).  These values hold on Zen 3/4/5, Golden Cove,
Raptor Cove and Skylake-SP alike (Agner Fog's tables, uops.info), so the
bound is microarchitecture-neutral for the ops it accepts.

Non-destructive forms (VEX three-operand vector ops, BMI rorx/andn/shlx/
shrx/sarx, lea) read only their source operands; legacy two-operand forms
also read their destination.

With --loads, memory *source* operands are accepted: a load is a 5-cycle
source (L1 hit) that depends only on its address registers, so a load
addressed by the induction variable is correctly off every data
recurrence.  Stores remain a hard error, and loop selection skips loops
that store: a store can carry the recurrence through memory (store-to-load
forwarding), which this model does not track.

Usage:
    loop_latency.py FILE.s --function chacha20_core [--label .LBB8] [--loads] [--json]

Without --label the loop is the largest straight-line loop in FUNCTION: a
backward branch whose body contains no other branch (with --loads: and no
store).
"""
from __future__ import annotations

import argparse
import json
import re
import sys

LAT1 = {
    # SSE/AVX integer
    "paddd", "psubd", "pxor", "por", "pand", "pandn", "pslld", "psrld",
    "psllq", "psrlq", "pshufb", "pshufd", "paddq", "psubq",
    "vpaddd", "vpsubd", "vpxor", "vpor", "vpand", "vpandn", "vpslld",
    "vpsrld", "vpsllq", "vpsrlq", "vpshufb", "vpshufd", "vpaddq", "vpsubq",
    "vprold", "vprord", "vpternlogd",
    # GPR integer
    "addl", "addq", "subl", "subq", "xorl", "xorq", "orl", "orq", "andl",
    "andq", "roll", "rorl", "rolq", "rorq", "rorxl", "rorxq", "leal", "leaq",
    "incl", "incq", "decl", "decq", "notl", "notq", "negl", "negq",
    "shll", "shrl", "sarl", "shlq", "shrq", "sarq", "cmpl", "cmpq",
    "testl", "testq", "andnl", "andnq", "shlxl", "shlxq", "shrxl", "shrxq",
    "sarxl", "sarxq",
}
# Three-operand forms that do not read their destination.
NONDESTRUCTIVE = {"rorxl", "rorxq", "andnl", "andnq", "shlxl", "shlxq",
                  "shrxl", "shrxq", "sarxl", "sarxq", "leal", "leaq"}
LOAD_LATENCY = 5
MOVES = {"movdqa", "movaps", "vmovdqa", "vmovaps", "movl", "movq", "vmovdqu",
         "movdqu", "vmovups", "movups"}
BRANCH = re.compile(r"^j[a-z]+$")
REG = re.compile(r"%([a-z0-9]+)")
# Registers whose low parts alias (eax/rax/ax/al ...) share one ready time.
ALIAS = {}
for base in ("ax", "bx", "cx", "dx", "si", "di", "bp", "sp"):
    for form in ("r" + base, "e" + base, base):
        ALIAS[form] = base
for n in range(8, 16):
    for suffix in ("", "d", "w", "b"):
        ALIAS[f"r{n}{suffix}"] = f"r{n}"
for n in range(32):
    for pre in ("xmm", "ymm", "zmm"):
        ALIAS[f"{pre}{n}"] = f"v{n}"


def canon(reg: str) -> str:
    return ALIAS.get(reg, reg)


def split_operands(text: str) -> list[str]:
    out, depth, cur = [], 0, ""
    for ch in text:
        if ch == "(":
            depth += 1
        elif ch == ")":
            depth -= 1
        if ch == "," and depth == 0:
            out.append(cur.strip())
            cur = ""
        else:
            cur += ch
    if cur.strip():
        out.append(cur.strip())
    return out


def function_lines(lines: list[str], func: str) -> list[str]:
    start = next((i for i, l in enumerate(lines) if l.strip() == f"{func}:"), None)
    if start is None:
        sys.exit(f"function {func!r} not found")
    end = len(lines)
    for i in range(start + 1, len(lines)):
        if lines[i].strip().startswith(".size") or lines[i].strip() == ".cfi_endproc":
            end = i
            break
    return lines[start:end]


def is_store(text: str) -> bool:
    parts = text.split(None, 1)
    if len(parts) < 2 or parts[0].startswith(("lea", "cmp", "test")):
        return False
    ops = split_operands(parts[1])
    return len(ops) >= 2 and "(" in ops[-1]


def pick_loop(body: list[str], label: str | None, skip_stores: bool = False) -> list[str]:
    labels = {}
    for i, l in enumerate(body):
        m = re.match(r"^\s*(\.?L[\w.$]+):", l)
        if m:
            labels[m.group(1)] = i
    best = None
    for i, l in enumerate(body):
        parts = l.split()
        if len(parts) == 2 and BRANCH.match(parts[0]) and parts[1] in labels:
            s = labels[parts[1]]
            if s < i and (label is None or parts[1] == label):
                # Only straight-line bodies: the model is a single-path
                # recurrence, so a loop enclosing another branch (an outer
                # loop, a guard) is not a candidate.
                inner = [b.split() for b in body[s + 1:i]]
                if any(b and BRANCH.match(b[0]) for b in inner):
                    continue
                if skip_stores and any(is_store(b.split("#", 1)[0].strip()) for b in body[s + 1:i]):
                    continue
                if best is None or i - s > best[1] - best[0]:
                    best = (s, i)
    if best is None:
        sys.exit("no backward-branch loop found" + (f" at {label}" if label else ""))
    s, e = best
    insns = []
    for l in body[s + 1:e]:
        t = l.split("#", 1)[0].strip()
        if t and not t.startswith(".") and not re.match(r"^\.?L[\w.$]+:", t):
            insns.append(t)
    return insns


def analyse(insns: list[str], iterations: int = 64, loads: bool = False) -> dict:
    ready: dict[str, int] = {}
    ends = []
    for _ in range(iterations):
        for text in insns:
            parts = text.split(None, 1)
            mnem = parts[0]
            ops = split_operands(parts[1]) if len(parts) > 1 else []
            is_lea = mnem.startswith("lea")
            mem = [o for o in ops if "(" in o] if not is_lea else []
            load_t = None
            if mem:
                if not loads:
                    sys.exit(f"memory operand in a latency-model loop: {text!r}")
                if is_store(text):
                    sys.exit(f"store in a latency-model loop: {text!r}")
                addr = [canon(r) for r in REG.findall(mem[0])]
                load_t = max((ready.get(r, 0) for r in addr), default=0) + LOAD_LATENCY
            if mnem in MOVES and len(ops) == 2 and ops[1].startswith("%"):
                if ops[0].startswith("%"):
                    ready[canon(ops[1][1:])] = ready.get(canon(ops[0][1:]), 0)
                    continue
                if load_t is not None:
                    ready[canon(ops[1][1:])] = load_t
                    continue
                if ops[0].startswith("$"):
                    ready[canon(ops[1][1:])] = 0  # immediate: no inputs
                    continue
            if mnem not in LAT1:
                sys.exit(f"unknown mnemonic {mnem!r} in {text!r}: extend the table")
            reg_ops = [o for o in ops if "(" not in o or is_lea]
            dest_op = ops[-1] if ops else ""
            dest = canon(REG.findall(dest_op)[0]) if dest_op.startswith("%") else None
            vex3 = mnem.startswith("v") and len(ops) == 3
            if vex3 or mnem in NONDESTRUCTIVE:
                src_ops = [o for o in reg_ops if o is not dest_op]
            else:
                src_ops = reg_ops  # legacy 2-op: dest is a source
            srcs = [canon(r) for o in src_ops for r in REG.findall(o)]
            if mnem.startswith("cmp") or mnem.startswith("test"):
                dest = "flags"
            t = max([ready.get(r, 0) for r in srcs] + ([load_t] if load_t is not None else []),
                    default=0) + 1
            if dest:
                ready[dest] = t
        ends.append(max(ready.values(), default=0))
    warm = iterations // 4
    per_iter = (ends[-1] - ends[warm]) / (iterations - 1 - warm)
    return {"instructions": len(insns), "cycles_per_iteration": per_iter}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("asm")
    ap.add_argument("--function", required=True)
    ap.add_argument("--label")
    ap.add_argument("--loads", action="store_true",
                    help="accept memory source operands (5-cycle loads); skip storing loops")
    ap.add_argument("--json", action="store_true")
    args = ap.parse_args()
    lines = open(args.asm, encoding="utf-8").read().splitlines()
    insns = pick_loop(function_lines(lines, args.function), args.label, skip_stores=args.loads)
    res = analyse(insns, loads=args.loads)
    if args.json:
        print(json.dumps(res))
    else:
        print(f"{res['instructions']} instructions, recurrence bound "
              f"{res['cycles_per_iteration']:.2f} cycles/iteration")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
