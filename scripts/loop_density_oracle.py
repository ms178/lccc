#!/usr/bin/env python3
"""Rank loop kernels by STEADY-STATE loop density: LCCC vs the competition.

Why this exists
---------------
`codegen_oracle.py --rank` compares STATIC whole-function instruction counts.
That metric is the right one for straight-line and branchy code, and the wrong
one for loop kernels, which is most of the benchmark corpus: a compiler that
refuses to vectorize emits one tight scalar loop and "wins" the static column
while doing 32x less work per instruction.  `scripts/hot_loop_metric.py`
measures the honest quantity -- instructions retired per input byte in the
steady-state (innermost, non-composite) loop -- but only one function at a
time.

This tool is the two of them joined: for every function of every corpus file
it measures LCCC's density and every oracle's density and ranks by the ratio,
so effort goes to the kernels we are genuinely behind on.

Measured example (matmul, `-O2 -march=x86-64-v3`), where the static rank and
the density rank disagree in SIGN, not just magnitude:

    static  (codegen_oracle --rank): lccc 84 insns vs gcc 21  -> "63 behind"
    density (this tool):             lccc 0.1172 insn/B vs gcc 0.1875
                                     -> lccc 1.60x AHEAD

Comparability rules (no silent apples-to-oranges)
-------------------------------------------------
* A function is compared only when LCCC and at least `--min-oracles` oracles
  emit it as a standalone symbol with a measurable innermost loop.  Different
  inlining decisions otherwise compare a whole inlined kernel against an
  out-of-line stub.
* A function whose steady-state loop is composite (contains another loop) or
  whose per-trip step cannot be recovered yields NO density: it is reported as
  unmeasured, never as 0.0 and never as a win.
* Ratios below 1.0 mean LCCC does less work per byte (better).

Usage::

    scripts/loop_density_oracle.py tests/benchmark/programs/*.c \\
        --json results/density.json --markdown results/density.md

    # same-binary A/B: measure LCCC against itself with a knob flipped
    CCC_NO_MAP_ZERO_REM=1 scripts/loop_density_oracle.py --local-only \\
        tests/benchmark/programs/*.c

Remote compiles go through `scripts/codegen_oracle.py`'s cache, so a survey
run after `--rank` costs no network requests and cannot be rate-limited.
"""
from __future__ import annotations

import argparse
import json
import os
import sys
from dataclasses import asdict, dataclass
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(REPO / "scripts"))

import codegen_oracle as oracle  # noqa: E402
import hot_loop_metric as H  # noqa: E402

DEFAULT_ORACLES = ("gcc16.2", "clang", "icc", "icx")


@dataclass
class Row:
    benchmark: str
    symbol: str
    lccc_insns: int
    lccc_bytes_per_trip: int
    lccc_density: float | None
    lccc_vector_bits: int
    lccc_loop: str
    best_oracle: str | None
    best_density: float | None
    ratio: float | None          # lccc_density / best_density; <1.0 is a win
    oracles: dict[str, float | None]
    note: str = ""


def _measurable(result: dict) -> bool:
    return "error" not in result and result.get("insns_per_byte") is not None


def measure_all(lines: list[str]) -> dict[str, dict]:
    """Density of every function symbol in one assembly listing."""
    out: dict[str, dict] = {}
    # `_split_function_bodies` needs the whole listing; reuse it for the
    # symbol list so this tool and codegen_oracle agree on what counts as a
    # function (a non-dot label; `.L*` stays inside its enclosing function).
    # The label line is re-prepended because `analyse_lines` expects the
    # symbol to be present in the listing it is given.
    for symbol, body in oracle._split_function_bodies(lines).items():
        result = H.analyse_lines([f"{symbol}:"] + body, symbol, file="")
        if _measurable(result):
            out[symbol] = result
    return out


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(
        description="rank loop kernels by steady-state density (LCCC vs oracles)")
    ap.add_argument("sources", nargs="+", type=Path)
    ap.add_argument("--local", default=str(REPO / "target" / "fastbuild" / "lccc"))
    ap.add_argument("--flags", default="-O2 -march=x86-64-v3")
    ap.add_argument("--local-flags")
    ap.add_argument("--oracles", default=",".join(DEFAULT_ORACLES))
    ap.add_argument("--local-only", action="store_true",
                    help="no remote compiles (same-binary A/B of LCCC)")
    ap.add_argument("--min-oracles", type=int, default=1,
                    help="oracles that must emit a comparable symbol (default 1)")
    ap.add_argument("--top", type=int)
    ap.add_argument("--json", type=Path)
    ap.add_argument("--markdown", type=Path)
    args = ap.parse_args(argv)

    oracle_names = [o for o in (s.strip() for s in args.oracles.split(","))
                    if o and not args.local_only]
    rows: list[Row] = []
    for source in args.sources:
        source_text = source.read_text(errors="replace")
        try:
            local_lines = oracle._local_compile(args.local, source,
                                                args.local_flags or args.flags)
        except Exception as exc:  # noqa: BLE001 - a survey reports, not aborts
            print(f"{source}: local compile failed: {exc}", file=sys.stderr)
            continue
        local = measure_all(local_lines)
        remote: dict[str, dict[str, dict]] = {}
        for name in oracle_names:
            try:
                remote[name] = measure_all(
                    oracle._compile_remote(name, source_text, args.flags))
            except Exception as exc:  # noqa: BLE001
                print(f"{source}: {name}: {exc}", file=sys.stderr)
        for symbol, lr in sorted(local.items()):
            per_oracle: dict[str, float | None] = {}
            for name, table in remote.items():
                rr = table.get(symbol)
                per_oracle[name] = (rr["insns_per_byte"] if rr else None)
            measured = {k: v for k, v in per_oracle.items() if v is not None}
            best_name = min(measured, key=lambda k: measured[k]) if measured else None
            best = measured[best_name] if best_name else None
            ratio = (lr["insns_per_byte"] / best) if best else None
            note = ""
            if not args.local_only and len(measured) < args.min_oracles:
                note = (f"only {len(measured)} oracle(s) emit a comparable "
                        f"'{symbol}' (inlining differs): not ranked")
            rows.append(Row(
                benchmark=source.stem, symbol=symbol,
                lccc_insns=lr["insns"],
                lccc_bytes_per_trip=lr["bytes_per_trip"],
                lccc_density=lr["insns_per_byte"],
                lccc_vector_bits=lr["widest_vector_bits"],
                lccc_loop=lr["loop_label"],
                best_oracle=best_name, best_density=best, ratio=ratio,
                oracles=per_oracle, note=note,
            ))

    # Worst first: a ratio > 1 means LCCC retires more instructions per byte.
    ranked = [r for r in rows if r.ratio is not None and not r.note]
    unranked = [r for r in rows if r.ratio is None or r.note]
    ranked.sort(key=lambda r: (-(r.ratio or 0), r.benchmark, r.symbol))

    width = max((len(r.benchmark) for r in rows), default=9)
    print(f"{'ratio':>6}  {'benchmark':<{width}}  {'function':<22} "
          f"{'lccc insn/B':>11}  {'best':>11}  oracle")
    print("-" * (width + 66))
    shown = ranked if args.top is None else ranked[:args.top]
    for r in shown:
        print(f"{r.ratio:>6.2f}  {r.benchmark:<{width}}  {r.symbol:<22} "
              f"{r.lccc_density:>11.4f}  {r.best_density:>11.4f}  "
              f"{r.best_oracle} ({r.lccc_insns} insn / {r.lccc_bytes_per_trip} B)")
    wins = [r for r in ranked if r.ratio < 1.0]
    print("-" * (width + 66))
    print(f"compared: {len(ranked)} kernels | LCCC ahead on {len(wins)} | "
          f"behind on {len(ranked) - len(wins)} | uncomparable: {len(unranked)}")
    if ranked:
        geo = 1.0
        for r in ranked:
            geo *= r.ratio
        print(f"geometric mean of LCCC/best density ratio: "
              f"{geo ** (1.0 / len(ranked)):.4f}  (lower is better)")

    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        tmp = args.json.with_suffix(f".tmp.{os.getpid()}")
        tmp.write_text(json.dumps({
            "schema": 1,
            "flags": args.flags,
            "local_flags": args.local_flags or args.flags,
            "oracles": oracle_names,
            "ranked": [asdict(r) for r in ranked],
            "unranked": [asdict(r) for r in unranked],
        }, indent=2) + "\n")
        tmp.replace(args.json)
        print(f"wrote {args.json}")
    if args.markdown:
        args.markdown.parent.mkdir(parents=True, exist_ok=True)
        lines = [
            f"# Loop-density survey ({args.flags})",
            "",
            "Ratio = LCCC steady-state instructions per byte / best oracle's. "
            "**< 1.0 means LCCC does less work per byte.**",
            "",
            "| ratio | benchmark | function | LCCC insn/B | best insn/B | best oracle | LCCC loop |",
            "|---|---|---|---|---|---|---|",
        ]
        for r in ranked:
            lines.append(
                f"| {r.ratio:.2f} | {r.benchmark} | `{r.symbol}` | "
                f"{r.lccc_density:.4f} | {r.best_density:.4f} | {r.best_oracle} | "
                f"{r.lccc_insns}/{r.lccc_bytes_per_trip} B |")
        args.markdown.write_text("\n".join(lines) + "\n")
        print(f"wrote {args.markdown}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
