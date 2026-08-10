#!/usr/bin/env python3
"""Deterministic defined-behavior differential fuzzer for LCCC x86-64.

Each generated translation unit exercises fixed-width arithmetic, CFG joins,
array addressing, structs/bitfields, global state, calls, volatile accesses,
post-increment/decrement stack homes, and a limited __int128 data path.  It
compares the complete stdout and exit status of LCCC against a reference C
compiler.  The generator deliberately avoids signed overflow, invalid shifts,
division by zero, invalid pointers, and unspecified argument evaluation order.

Usage:
  differential_fuzz.py --ccc /path/to/lccc --gcc /usr/bin/gcc \
      --seeds 0:120 --levels O0,O3,Os --jobs 2 --out /tmp/results
"""
from __future__ import annotations

import argparse
import concurrent.futures as futures
import json
import os
import random
import subprocess
import sys
from dataclasses import dataclass, asdict
from pathlib import Path
from typing import Iterable

MASK64 = (1 << 64) - 1


def u64_literal(value: int) -> str:
    return f"UINT64_C(0x{value & MASK64:016x})"


def u32_literal(value: int) -> str:
    return f"UINT32_C(0x{value & 0xffffffff:08x})"


def generate(seed: int) -> str:
    r = random.Random(seed)
    c = [
        "#include <stdint.h>",
        "#include <stdio.h>",
        "#include <inttypes.h>",
        "",
        "static uint64_t global_a;",
        "static uint32_t global_b;",
        "static volatile uint32_t observed;",
        "",
        "struct pair { uint32_t lo; uint32_t hi; };",
        "struct bits { unsigned a:3; unsigned b:5; unsigned c:8; };",
        "",
        "static uint64_t rotl64(uint64_t x, unsigned n) {",
        "  n &= 63u; return n ? ((x << n) | (x >> ((64u - n) & 63u))) : x;",
        "}",
        "static uint64_t mix(uint64_t x, uint64_t y, unsigned n) {",
        "  x ^= rotl64(y + UINT64_C(0x9e3779b97f4a7c15), n);",
        "  x *= UINT64_C(0xbf58476d1ce4e5b9);",
        "  x ^= x >> 29; return x;",
        "}",
        "static struct pair step_pair(struct pair p, uint32_t x) {",
        "  p.lo = (p.lo + x) ^ (p.hi >> 3);",
        "  p.hi = (p.hi * UINT32_C(1664525)) + UINT32_C(1013904223) + p.lo;",
        "  return p;",
        "}",
        "static uint64_t postdec_path(uint32_t n, uint64_t seed) {",
        "  uint64_t sum = seed;",
        "  do { sum = mix(sum, (uint64_t)n + UINT64_C(17), n); } while (n-- != 0);",
        "  return sum;",
        "}",
        "static uint64_t wide_path(uint64_t a, uint64_t b) {",
        "  __int128 x = ((__int128)(uint64_t)a << 32) | (uint32_t)b;",
        "  x = x * 3 + 7;",
        "  return (uint64_t)x ^ (uint64_t)(x >> 64);",
        "}",
        "",
        "int main(void) {",
        f"  uint64_t acc = {u64_literal(r.getrandbits(64))};",
        f"  uint64_t salt = {u64_literal(r.getrandbits(64))};",
        "  uint32_t a[16];",
        "  struct pair p = { UINT32_C(1), UINT32_C(2) };",
        "  struct bits bf = { 0, 0, 0 };",
        "  for (unsigned i = 0; i < 16; ++i) a[i] = (uint32_t)(acc >> (i & 31u)) ^ (i * UINT32_C(0x45d9f3b));",
    ]
    # A sequence of defined operations and CFG diamonds.  Constants ensure
    # outputs vary enough to avoid trivial constant-fold-only coverage.
    for i in range(r.randint(10, 22)):
        op = r.randrange(7)
        x = r.getrandbits(64)
        y = r.getrandbits(64)
        idx = r.randrange(16)
        shift = r.randrange(64)
        if op == 0:
            c.append(f"  acc = mix(acc + {u64_literal(x)}, salt ^ {u64_literal(y)}, {shift}u);")
        elif op == 1:
            c.append(f"  a[{idx}] ^= (uint32_t)(acc >> {shift}u); acc += a[{idx}];")
        elif op == 2:
            c.append(f"  if ((acc ^ {u64_literal(x)}) & UINT64_C(1)) acc ^= rotl64(salt, {shift}u); else acc += {u64_literal(y)};")
        elif op == 3:
            c.append(f"  p = step_pair(p, (uint32_t)(acc + {u32_literal(x)})); acc ^= ((uint64_t)p.hi << 32) | p.lo;")
        elif op == 4:
            c.append(f"  bf.a = (unsigned)(acc >> {shift}u); bf.b = (unsigned)(salt >> {(shift + 7) & 63}u); bf.c = bf.a + bf.b; acc += (uint64_t)(bf.a | (bf.b << 3) | (bf.c << 8));")
        elif op == 5:
            # bounded signed arithmetic only; no signed overflow.
            sa = r.randrange(-20000, 20001)
            sb = r.randrange(-20000, 20001)
            c.append(f"  {{ int32_t sx = {sa}; int32_t sy = {sb}; int32_t z = sx * 3 + sy * 5; acc ^= (uint64_t)(int64_t)z; }}")
        else:
            c.append(f"  global_a ^= acc + {u64_literal(x)}; global_b += (uint32_t)(salt >> {shift}u); acc = mix(acc, global_a ^ global_b, {shift}u);")
    # Mandatory volatile and postdec paths.  Every volatile read is observable
    # and should not be forwarded/reordered by stack-slot peepholes.
    n = r.randrange(0, 12)
    c += [
        f"  observed = (uint32_t)(acc ^ {u32_literal(r.getrandbits(32))});",
        "  { volatile uint32_t local_v = observed; local_v ^= (uint32_t)salt; observed = local_v; acc ^= observed; }",
        f"  acc ^= postdec_path({n}u, salt);",
        "  acc ^= wide_path(acc, salt);",
        "  for (unsigned j = 0; j < 16; ++j) {",
        "    unsigned k = (unsigned)((acc + j) & 15u);",
        "    a[k] = (uint32_t)mix(a[k], acc ^ j, j);",
        "    acc ^= ((uint64_t)a[k] << ((j & 7u) * 8u));",
        "  }",
        "  printf(\"%016\" PRIx64 \" %08\" PRIx32 \"\\n\", acc, observed);",
        "  return 0;",
        "}",
    ]
    return "\n".join(c) + "\n"


@dataclass
class Result:
    seed: int
    level: str
    status: str
    lccc_rc: int | str
    gcc_rc: int | str
    lccc_out: str
    gcc_out: str
    detail: str = ""


def invoke(cmd: list[str], timeout: int) -> tuple[int | str, str, str]:
    try:
        p = subprocess.run(cmd, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                           timeout=timeout)
        return p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired:
        return "TIMEOUT", "", "timeout"
    except OSError as e:
        return "ERROR", "", repr(e)


def one_case(args: tuple[int, str, str, str, Path]) -> Result:
    seed, level, ccc, gcc, out = args
    case = out / f"seed_{seed:05d}_{level}"
    case.mkdir(parents=True, exist_ok=True)
    src = case / "case.c"
    src.write_text(generate(seed))
    ccc_bin = case / "lccc.bin"
    gcc_bin = case / "gcc.bin"
    lflag = f"-{level}"
    glevel = "-Os" if level == "Oz" else lflag
    common = ["-std=gnu11", "-w", "-march=raptorlake", "-mtune=raptorlake", "-fomit-frame-pointer"]
    grc, _, gerr = invoke([gcc, glevel, *common, str(src), "-o", str(gcc_bin)], 45)
    if grc != 0:
        return Result(seed, level, "reference_compile_failure", "NA", grc, "", "", gerr[-500:])
    lrc, _, lerr = invoke([ccc, lflag, *common, str(src), "-o", str(ccc_bin)], 60)
    if lrc != 0:
        return Result(seed, level, "lccc_compile_failure", lrc, grc, "", "", lerr[-1000:])
    grun, gout, gerr = invoke([str(gcc_bin)], 10)
    lrun, lout, lerr = invoke([str(ccc_bin)], 10)
    if grun == lrun and gout == lout:
        # Keep the successful corpus compact; source can be reproduced from seed.
        for p in (ccc_bin, gcc_bin):
            p.unlink(missing_ok=True)
        src.unlink(missing_ok=True)
        try:
            case.rmdir()
        except OSError:
            pass
        return Result(seed, level, "pass", lrun, grun, lout, gout)
    return Result(seed, level, "mismatch", lrun, grun, lout, gout,
                  f"lccc_stderr={lerr[-500:]}\ngcc_stderr={gerr[-500:]}")


def parse_seeds(spec: str) -> range:
    if ":" in spec:
        lo, hi = spec.split(":", 1)
        return range(int(lo), int(hi))
    return range(int(spec), int(spec) + 1)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--ccc", required=True)
    ap.add_argument("--gcc", required=True)
    ap.add_argument("--seeds", default="0:100")
    ap.add_argument("--levels", default="O0,O3,Os")
    ap.add_argument("--jobs", type=int, default=1)
    ap.add_argument("--out", type=Path, required=True)
    ns = ap.parse_args()
    ns.out.mkdir(parents=True, exist_ok=True)
    levels = [x.strip() for x in ns.levels.split(",") if x.strip()]
    jobs = max(1, ns.jobs)
    cases = [(seed, level, ns.ccc, ns.gcc, ns.out) for level in levels for seed in parse_seeds(ns.seeds)]
    failures: list[Result] = []
    counts: dict[str, int] = {}
    with futures.ThreadPoolExecutor(max_workers=jobs) as pool:
        for result in pool.map(one_case, cases):
            counts[result.status] = counts.get(result.status, 0) + 1
            if result.status != "pass":
                failures.append(result)
                print(f"FAIL seed={result.seed} level={result.level} status={result.status}", flush=True)
            else:
                print(f"PASS seed={result.seed} level={result.level}", flush=True)
    report = {
        "ccc": ns.ccc, "gcc": ns.gcc, "seeds": ns.seeds, "levels": levels,
        "jobs": jobs, "cases": len(cases), "counts": counts,
        "failures": [asdict(x) for x in failures],
    }
    (ns.out / "summary.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(json.dumps(report, indent=2, sort_keys=True))
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
