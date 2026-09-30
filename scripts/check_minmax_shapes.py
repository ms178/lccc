#!/usr/bin/env python3
"""MINMAX-1 shape census: which min/max reductions lower to packed ops.

`codegen_oracle.py --rank` reports static instruction counts, which is the
wrong metric for a loop (a compiler that refuses to vectorize emits one tight
scalar loop and "wins"), and it cannot see *which* instruction family the loop
uses.  This tool measures the STEADY-STATE loop of every function in the
shape fixture with `hot_loop_metric.py` (dominator-based loop identification,
innermost non-composite loops only) and asserts each function's group:

* VECTORIZED — the steady-state loop must consume at least one 256-bit vector
  per trip (`bytes_per_trip >= 32` and a YMM operand). `vminss`/`vmaxss` are
  128-bit and process 4 bytes: scalar, not packed.
* REFUSED — the loop must stay scalar. This is the important half: modelling a
  multi-accumulator loop with a single-accumulator pattern produced
  `sum == 0` for every n below the vector width, so a shape this transform
  does not model must be *seen* to stay scalar, not assumed to.

A capability increase (unsigned min/max, 16-bit lanes, multi-accumulator) is
welcome and should move a function between groups — deliberately, with the
differential gate proving correctness, not by drifting past this check.

Usage::

    scripts/check_minmax_shapes.py --ccc target/fastbuild/lccc \\
        --source tests/regression/minmax_shapes/minmax_shapes.c
"""
from __future__ import annotations

import argparse
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(REPO / "scripts"))

import codegen_oracle as oracle  # noqa: E402
import hot_loop_metric as H  # noqa: E402

# Function -> expected group.
VECTORIZED = {
    "max_i32",
    "min_i32",
    "max_swapped",
    "min_le",
    "max_ge",
    "max_start1",
    "max_const_init",
}
REFUSED = {
    "max_u32",          # unsigned compare: vpmaxsd/vpminsd are signed-only
    "max_i16",          # 16-bit lanes: no packed min/max there yet
    "min_i16",
    "min_f32",          # NaN semantics: FP min/max needs -ffast-math
    "max_guarded",      # `continue`: not every iteration contributes
    "max_with_sum",     # second accumulator (independent sum)
    "minmax_pair",      # second accumulator (min AND max)
}

# A packed 8xI32 loop processes 32 bytes per trip; `10 / 32 = 0.3125`.
MAX_DENSITY_VECTORIZED = 0.5


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--ccc", default=str(REPO / "target" / "fastbuild" / "lccc"))
    ap.add_argument("--source",
                    default=str(REPO / "tests" / "regression" / "minmax_shapes"
                                / "minmax_shapes.c"))
    ap.add_argument("--flags", default="-O3 -march=x86-64-v3")
    args = ap.parse_args()

    source = Path(args.source)
    with tempfile.TemporaryDirectory() as tmp:
        out = Path(tmp) / "shapes.s"
        cmd = [args.ccc, *args.flags.split(), "-S", str(source), "-o", str(out)]
        proc = subprocess.run(cmd, capture_output=True, text=True)
        if proc.returncode:
            print(f"check_minmax_shapes: compile failed: {' '.join(cmd)}\n"
                  f"{proc.stderr}", file=sys.stderr)
            return 1
        lines = out.read_text(errors="replace").splitlines()

    bodies = oracle._split_function_bodies(lines)
    fail = 0
    print(f"{'function':<16} {'group':<12} {'insn/B':>8} {'B/trip':>7} {'vec':>6}  verdict")
    print("-" * 62)
    for name in sorted(VECTORIZED | REFUSED):
        want_vector = name in VECTORIZED
        if name not in bodies:
            print(f"{name:<16} {'vectorized' if want_vector else 'refused':<12} "
                  f"{'--':>8} {'--':>7} {'--':>6}  FAIL: function not emitted")
            fail += 1
            continue
        res = H.analyse_lines([f"{name}:"] + bodies[name], name, file=str(source))
        if "error" in res:
            print(f"{name:<16} {'vectorized' if want_vector else 'refused':<12} "
                  f"{'--':>8} {'--':>7} {'--':>6}  FAIL: {res['error']}")
            fail += 1
            continue
        packed = res["bytes_per_trip"] >= 32 and res["widest_vector_bits"] >= 256
        ok = packed == want_vector
        if packed and want_vector and res["insns_per_byte"] > MAX_DENSITY_VECTORIZED:
            ok = False
        print(f"{name:<16} {'vectorized' if want_vector else 'refused':<12} "
              f"{res['insns_per_byte']:>8.4f} {res['bytes_per_trip']:>7} "
              f"{res['widest_vector_bits']:>5}b  {'PASS' if ok else 'FAIL'}")
        if not ok:
            print(f"    expected {'packed 256-bit' if want_vector else 'scalar'} "
                  f"steady-state loop, got {res['insns']} insns / "
                  f"{res['bytes_per_trip']} B, {res['widest_vector_bits']}b "
                  f"({res['step_evidence']})", file=sys.stderr)
            fail += 1
    print("-" * 62)
    if fail:
        print(f"check_minmax_shapes: FAIL ({fail} shape(s))", file=sys.stderr)
        return 1
    print(f"check_minmax_shapes: PASS ({len(VECTORIZED | REFUSED)} shapes)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
