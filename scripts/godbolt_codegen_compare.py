#!/usr/bin/env python3
"""Multi-compiler code-generation oracle: LCCC vs GCC/Clang/ICC/ICX on one corpus.

This is the research program's "compare codegen between all listed compilers"
tool, derived from :mod:`godbolt` the way the standing order asks: it reuses
godbolt's compile path and ``codegen_oracle``'s function splitter and
instruction statistics, and adds the one thing the existing survey tools do not
have -- **per-compiler flag translation**, plus a runtime arm that cannot be
gamed by refusing a transformation.

Why flag translation is the whole point
---------------------------------------
``codegen_oracle.py`` takes ONE ``--flags`` string and hands it to every
compiler verbatim.  That is correct only for the GCC-compatible subset, and it
silently corrupts the most important comparison in the corpus: Intel ICC
Classic 2021.10 predates ``-march=x86-64-v3`` and ``-march=raptorlake`` and
answers those with

    icc: command line warning #10148: option '-march=x86-64-v3' not supported

and then compiles WITHOUT the requested ISA.  A measured ICC row that looks
2.7x larger than LCCC's may simply be SSE2-only code, and a "LCCC beats ICX"
claim made against an SSE2 ICC build is worthless.  This tool therefore
compiles the *ISA intent, not the spelling*:

    -march=x86-64-v3 / raptorlake / alderlake / znver3
                            -> icc: -march=core-avx2   (Intel's AVX2 level)
    -march=x86-64-v4 / icelake-* / znver4
                            -> icc: -march=core-avx512
    -march=native           -> icc: dropped, and the row is LABELLED a
                            lower bound (native is meaningless off-host)
    anything else           -> verbatim

Every translation lands in the printed notes and in the JSON manifest, so no
table can be read without knowing which flags produced it.  ``--strict-flags``
restores verbatim behavior; ``--icc-march=`` pins an explicit level.

What is measured (and what is not)
----------------------------------
``--mode static`` (default) compares static instruction counts from
``codegen_oracle._stats`` -- the SAME implementation the survey engine and
``oracle_asm.py`` use, so a number here is comparable with the repository's
existing tables.  It is a *screening* metric: a compiler that refuses to
vectorize emits a tight scalar loop and "wins" the column while doing 8x the
work per byte, so loop kernels must be ranked with ``scripts/hot_loop_metric.py``
or the runtime arm.

``--mode runtime`` compiles AND executes the source on Compiler Explorer and
reports the relative execution time the CE runner measured.  It is the metric
that cannot be gamed by declining a transformation; the price is CE execution
noise, so it is reported as a ratio with the raw numbers retained, never as a
certified time.

Exit status
-----------
A survey is not a gate: the default exit is non-zero only when a *requested*
compiler or function fails.  ``--fail-on-behind [PCT]`` turns the ranking into
a gate (bare flag = 1%), which is what the low-risk/high-reward sweep uses.

Examples
--------
Rank the zlib-ng extracts, worst LCCC gap first::

    scripts/godbolt_codegen_compare.py tests/benchmark/programs/zlib_ng_*.c \\
        --flags '-O2 -march=x86-64-v3'

One function with reviewable assembly::

    scripts/godbolt_codegen_compare.py hot.c --function crc32_update \\
        --flags '-O3 -march=raptorlake' --artifact-dir results/crc

Exploratory whole-program timings on remote executors (not a host gate)::

    scripts/godbolt_codegen_compare.py matmul.c --mode runtime

Re-run offline against the cache and diff against a saved manifest::

    scripts/godbolt_codegen_compare.py corpus/*.c --offline --baseline before.json
"""
from __future__ import annotations

import argparse
import concurrent.futures
import json
import re
import statistics
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
REPO_ROOT = HERE.parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(REPO_ROOT))

import codegen_oracle as co  # noqa: E402
import godbolt  # noqa: E402

DEFAULT_ORACLES = ("gcc16.2", "clang", "icc", "icx")

# ICC Classic's vocabulary. Keys are ``-march=`` values accepted by GCC/Clang
# but not (or not with the same codegen) by ICC 2021.10.
ICC_MARCH_FALLBACK = {
    "x86-64-v3": "core-avx2",
    "x86-64-v4": "core-avx512",
    "raptorlake": "core-avx2",
    "alderlake": "core-avx2",
    "rocketlake": "core-avx2",
    "tigerlake": "core-avx2",
    "icelake-client": "core-avx512",
    "icelake-server": "core-avx512",
    "sapphirerapids": "core-avx512",
    "znver3": "core-avx2",
    "znver4": "core-avx512",
}
# ``-march=native`` has no off-host meaning; dropping it is honest only when the
# report says so, which is why the note is mandatory rather than optional.
ICC_REFUSED_MARCH = {"native"}

_MARCH = re.compile(r"-march=(\S+)")


@dataclass
class FlagPlan:
    """One logical flag set, resolved to each compiler's spelling."""

    requested: str
    per_compiler: dict[str, str] = field(default_factory=dict)
    notes: list[str] = field(default_factory=list)

    def for_compiler(self, name: str) -> str:
        return self.per_compiler.get(name, self.requested)


def translate_flags(flags: str, compilers: list[str], *,
                    icc_march: str | None = None,
                    strict: bool = False) -> FlagPlan:
    """Resolve ``-march=`` per compiler. Every other flag is vendor-neutral."""
    plan = FlagPlan(flags, {c: flags for c in compilers})
    if strict:
        plan.notes.append("--strict-flags: verbatim for every compiler")
        return plan
    match = _MARCH.search(flags)
    if not match or "icc" not in compilers:
        return plan
    march = match.group(1)
    if icc_march:
        plan.per_compiler["icc"] = _MARCH.sub(f"-march={icc_march}", flags)
        plan.notes.append(f"icc: -march={march} -> -march={icc_march} (--icc-march)")
    elif march in ICC_REFUSED_MARCH:
        plan.per_compiler["icc"] = re.sub(r"\s*-march=\S+", "", flags)
        plan.notes.append(
            f"icc: -march={march} dropped (no off-host meaning); ICC row is a "
            "LOWER BOUND at its default ISA")
    elif march in ICC_MARCH_FALLBACK:
        plan.per_compiler["icc"] = _MARCH.sub(f"-march={ICC_MARCH_FALLBACK[march]}", flags)
        plan.notes.append(
            f"icc: -march={march} -> -march={ICC_MARCH_FALLBACK[march]} "
            "(same ISA level, ICC's name for it)")
    elif not march.startswith("core-"):
        plan.notes.append(
            f"icc: -march={march} passed through; ICC 2021.10 may reject it and "
            "then compile at its default ISA -- check the errors column")
    return plan


@dataclass
class Row:
    source: str
    function: str
    stats: dict[str, co.AsmStats]
    errors: dict[str, str]
    runtime: dict[str, float] = field(default_factory=dict)

    def _insns(self, name: str) -> int | None:
        stat = self.stats.get(name)
        return stat.instructions if stat else None

    @property
    def best_name(self) -> str:
        if self.runtime:
            return min(self.runtime, key=self.runtime.get)
        live = {n: self._insns(n) for n in self.stats if self._insns(n)}
        return min(live, key=lambda n: live[n]) if live else ""

    @property
    def lccc_over_best(self) -> float | None:
        mine = self._insns("lccc")
        others = [self._insns(n) for n in self.stats
                  if n != "lccc" and self._insns(n)]
        if not mine or not others:
            return None
        return mine / min(others)

    @property
    def lccc_runtime_over_best(self) -> float | None:
        mine = self.runtime.get("lccc")
        others = [v for n, v in self.runtime.items() if n != "lccc" and v]
        if not mine or not others:
            return None
        return mine / min(others)

    def to_dict(self) -> dict[str, Any]:
        return {
            "source": self.source,
            "function": self.function,
            "best": self.best_name,
            "lccc_over_best": self.lccc_over_best,
            "lccc_runtime_over_best": self.lccc_runtime_over_best,
            "stats": {n: s.__dict__ for n, s in self.stats.items()},
            "runtime": self.runtime,
            "errors": self.errors,
        }


def _remote_lines(compiler: str, source: str, flags: str) -> tuple[list[str], str]:
    try:
        return co._compile_remote(compiler, source, flags), ""
    except Exception as exc:  # noqa: BLE001 - one vendor failing must not abort
        return [], f"{type(exc).__name__}: {exc}"


def _local_lines(executable: str, source: Path, flags: str) -> tuple[list[str], str]:
    try:
        return co._local_compile(executable, source, flags), ""
    except Exception as exc:  # noqa: BLE001
        return [], f"{type(exc).__name__}: {exc}"


def _ce_runtime(compiler: str, source: str, flags: str) -> tuple[float | None, str]:
    """Compile+execute on CE; return (elapsed_seconds, error)."""
    try:
        cid = godbolt.resolve_compiler(compiler)
        flag_text = godbolt.compiler_flags(cid, flags)
        data = godbolt.compile_on_godbolt(cid, source, flag_text, intel=False,
                                          execute=True, raise_on_error=True)
    except Exception as exc:  # noqa: BLE001
        return None, f"{type(exc).__name__}: {exc}"
    if not data:
        return None, "no CE response"
    execution = data.get("execResult", data)
    if execution.get("code") != 0:
        return None, f"execution failed (code {execution.get('code')})"
    stdout = "\n".join(part.get("text", "") for part in execution.get("stdout", []))
    match = re.search(r"([0-9]+(?:\.[0-9]+)?)\s*(ms|s|us|ns)\b", stdout)
    if not match:
        return None, f"no timing in program output (got {stdout[:120]!r})"
    value = float(match.group(1))
    scale = {"s": 1.0, "ms": 1e-3, "us": 1e-6, "ns": 1e-9}[match.group(2)]
    return value * scale, ""


def compare_file(path: Path, *, local: str, oracles: list[str], plan: FlagPlan,
                 function: str | None, artifact_dir: Path | None,
                 mode: str, min_instructions: int = 1) -> list[Row]:
    text = path.read_text(errors="replace")
    asm: dict[str, list[str]] = {}
    errs: dict[str, str] = {}
    runtime: dict[str, float] = {}

    if mode == "runtime":
        for name in oracles:
            elapsed, err = _ce_runtime(name, text, plan.for_compiler(name))
            if elapsed is None:
                errs[name] = err
            else:
                runtime[name] = elapsed
        return [Row(path.name, "<program>", {}, errs, runtime)]

    if local:
        lines, err = _local_lines(local, path, plan.for_compiler("lccc"))
        asm["lccc"], errs["lccc"] = lines, err
    for name in oracles:
        cid = godbolt.COMPILER_ALIASES.get(name, name)
        lines, err = _remote_lines(cid, text, plan.for_compiler(name))
        asm[name], errs[name] = lines, err
    if artifact_dir is not None:
        artifact_dir.mkdir(parents=True, exist_ok=True)
        for name, lines in asm.items():
            if lines:
                (artifact_dir / f"{path.stem}.{name}.s").write_text("\n".join(lines) + "\n")

    # Split every listing the SAME way the survey splits it, so per-function
    # bodies are identical between the two tools.
    bodies: dict[str, dict[str, list[str]]] = {
        name: co._split_function_bodies(lines) for name, lines in asm.items() if lines
    }
    if function:
        targets = [function]
    else:
        # A non-dot label in an assembly listing is a symbol, not necessarily a
        # function: `.bss`/`.data` objects (`left`, `window`, `position_count`)
        # and jump-table targets are emitted exactly like code labels.  Keeping
        # them made the first version of this report rank ten globals as
        # "functions" with an instruction count of zero, i.e. the table's
        # population was wrong before its numbers were.  The discriminator is
        # behavioural rather than syntactic: a symbol is code when at least one
        # requested compiler actually emitted instructions for it.
        targets = []
        for name in ("lccc", *oracles):
            for fn, body in bodies.get(name, {}).items():
                if fn in targets:
                    continue
                if co._stats(body, "x86").instructions >= min_instructions:
                    targets.append(fn)
    if not targets:
        return [Row(path.name, "<none>", {}, errs or {"all": "no functions emitted"})]

    rows: list[Row] = []
    arch = "x86"
    for fn in targets:
        stats: dict[str, co.AsmStats] = {}
        row_errs: dict[str, str] = {}
        for name in ("lccc", *oracles):
            if name in errs and errs[name]:
                row_errs[name] = errs[name]
                continue
            body = bodies.get(name, {}).get(fn)
            if body is None:
                row_errs[name] = f"function '{fn}' not emitted (inlined/removed)"
                continue
            stats[name] = co._stats(body, arch)
        rows.append(Row(path.name, fn, stats, row_errs, dict(runtime)))
    return rows


def _fmt(value: float | None) -> str:
    return "-" if value is None else f"{value:.3f}"


def render(rows: list[Row], compilers: list[str], mode: str, out=sys.stdout) -> None:
    names = (["lccc", *compilers] if any("lccc" in r.stats or "lccc" in r.errors for r in rows)
             else list(compilers))
    header = f"{'function':<32}{'source':<26}" + "".join(f"{n:>9}" for n in names)
    header += f"{'best':>10}{'L/best':>8}"
    print(header, file=out, flush=True)
    print("-" * len(header), file=out, flush=True)
    key = (lambda r: r.lccc_runtime_over_best) if mode == "runtime" \
        else (lambda r: r.lccc_over_best)
    for row in sorted(rows, key=lambda r: -(key(r) or 0.0)):
        line = f"{row.function[:31]:<32}{row.source[:25]:<26}"
        for name in names:
            if mode == "runtime":
                value = row.runtime.get(name)
                cell = "-" if value is None else f"{value * 1e3:.1f}"
            else:
                cell = str(row._insns(name) if row._insns(name) is not None else "!")
            line += f"{cell:>9}"
        ratio = key(row)
        line += f"{row.best_name:>10}{_fmt(ratio):>8}"
        print(line, file=out, flush=True)
    if mode == "runtime":
        print("\n(runtime column = CE-measured program seconds x 1e3; "
              "best is the fastest oracle time, not the fewest instructions)",
              file=out)


def summarize(rows: list[Row], compilers: list[str], mode: str) -> dict[str, Any]:
    key = (lambda r: r.lccc_runtime_over_best) if mode == "runtime" \
        else (lambda r: r.lccc_over_best)
    ratios = [(r.function, key(r)) for r in rows if key(r) is not None]
    values = [v for _, v in ratios]
    ahead = sum(1 for v in values if v < 1.0)
    tie = sum(1 for v in values if abs(v - 1.0) < 1e-9)
    winners: dict[str, int] = {}
    for row in rows:
        if row.best_name:
            winners[row.best_name] = winners.get(row.best_name, 0) + 1
    return {
        "functions": len(rows),
        "ranked": len(values),
        "lccc_ahead": ahead,
        "lccc_tie": tie,
        "lccc_behind": len(values) - ahead - tie,
        "geomean_lccc_over_best": (
            statistics.geometric_mean(values) if values else None),
        "median_lccc_over_best": (
            statistics.median(values) if values else None),
        "best_compiler_counts": winners,
        "top_gaps": [{"function": f, "ratio": round(v, 4)}
                     for f, v in sorted(ratios, key=lambda t: -t[1])[:10]],
        "top_wins": [{"function": f, "ratio": round(v, 4)}
                     for f, v in sorted(ratios, key=lambda t: t[1])[:10]],
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("sources", nargs="+", type=Path)
    parser.add_argument("--local", default=str(REPO_ROOT / "target/fastbuild/lccc"),
                        help="local LCCC executable, or 'none' for oracles only")
    parser.add_argument("--oracles", default=",".join(DEFAULT_ORACLES))
    parser.add_argument("--flags", default="-O2 -march=x86-64-v3")
    parser.add_argument("--local-flags", help="flags for LCCC when they must differ")
    parser.add_argument("--function", help="restrict to one function")
    parser.add_argument("--mode", choices=("static", "runtime"), default="static",
                        help="static instruction counts (default) or CE runtime")
    parser.add_argument("--icc-march", help="explicit -march for ICC")
    parser.add_argument("--strict-flags", action="store_true",
                        help="pass --flags verbatim to every compiler")
    parser.add_argument("--offline", action="store_true",
                        help="refuse network: cache hits only")
    parser.add_argument("--jobs", type=int, default=2, help="parallel source files")
    parser.add_argument("--min-instructions", type=int, default=1, metavar="N",
                        help="only rank symbols with at least N instructions "
                             "(default 1; excludes .bss/.data labels)")
    parser.add_argument("--artifact-dir", type=Path)
    parser.add_argument("--json", type=Path)
    parser.add_argument("--md", type=Path)
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--fail-on-behind", nargs="?", type=float, const=1.0,
                        metavar="PCT",
                        help="exit 1 if any function lags the best oracle by >PCT%%")
    parser.add_argument("-v", "--verbose", action="store_true")
    args = parser.parse_args(argv)

    if args.mode == "runtime" and (args.function or args.fail_on_behind is not None):
        parser.error("remote program timings cannot rank functions or gate host LCCC performance")

    oracles = [o for o in (x.strip() for x in args.oracles.split(",")) if o]
    local = "" if args.local in ("", "none") else args.local
    if args.mode == "runtime":
        local = ""
    if local and not Path(local).exists():
        print(f"error: local compiler not found: {local}", file=sys.stderr)
        return 2

    plan = translate_flags(args.flags, oracles, icc_march=args.icc_march,
                           strict=args.strict_flags)
    if args.local_flags:
        plan.per_compiler["lccc"] = args.local_flags
    for note in plan.notes:
        print(f"note: {note}", file=sys.stderr)

    rows: list[Row] = []
    failures: list[str] = []

    def work(path: Path) -> list[Row]:
        godbolt.CACHE_ONLY.set(args.offline)
        return compare_file(path, local=local, oracles=oracles, plan=plan,
                            function=args.function, artifact_dir=args.artifact_dir,
                            mode=args.mode,
                            min_instructions=args.min_instructions)

    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, args.jobs)) as pool:
        futures = {pool.submit(work, p): p for p in args.sources}
        for future in concurrent.futures.as_completed(futures):
            path = futures[future]
            try:
                result = future.result()
            except Exception as exc:  # noqa: BLE001
                failures.append(f"{path.name}: {type(exc).__name__}: {exc}")
                continue
            for row in result:
                if row.errors and (not row.stats or args.function or args.fail_on_behind is not None):
                    failures.append(f"{path.name}::{row.function}: {row.errors}")
            rows.extend(result)

    render(rows, oracles, args.mode)
    summary = summarize(rows, oracles, args.mode)
    print(f"\nfunctions: {summary['functions']}  ranked: {summary['ranked']}  "
          f"LCCC ahead {summary['lccc_ahead']} / tie {summary['lccc_tie']} / "
          f"behind {summary['lccc_behind']}")
    print(f"LCCC/best  geomean = {_fmt(summary['geomean_lccc_over_best'])}  "
          f"median = {_fmt(summary['median_lccc_over_best'])}")
    if summary["best_compiler_counts"]:
        print("best-compiler wins: " + ", ".join(
            f"{k}={v}" for k, v in sorted(summary["best_compiler_counts"].items())))
    if summary["top_gaps"]:
        print("worst gaps: " + ", ".join(
            f"{g['function']}={g['ratio']:.3f}" for g in summary["top_gaps"][:6]))
    if summary["top_wins"]:
        print("best wins:  " + ", ".join(
            f"{g['function']}={g['ratio']:.3f}" for g in summary["top_wins"][:6]))

    if args.baseline and args.baseline.exists():
        before = json.loads(args.baseline.read_text())
        attr = "lccc_runtime_over_best" if args.mode == "runtime" else "lccc_over_best"
        prev = {(r["source"], r["function"]): r.get(attr)
                for r in before.get("rows", [])}
        deltas = []
        for row in rows:
            old = prev.get((row.source, row.function))
            new = row.lccc_runtime_over_best if args.mode == "runtime" else row.lccc_over_best
            if old and new:
                deltas.append((row.function, old, new))
        if deltas:
            print("\nvs baseline (negative = improvement):")
            for fn, old, new in sorted(deltas, key=lambda d: d[2] - d[1]):
                flag = "  <-- regression" if new - old > 0.02 else ""
                print(f"  {fn:<32}{old:.3f} -> {new:.3f} ({new - old:+.3f}){flag}")

    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps({
            "tool": "godbolt_codegen_compare",
            "mode": args.mode,
            "flags": {"requested": args.flags, "per_compiler": plan.per_compiler,
                      "translation_notes": plan.notes},
            "oracles": oracles,
            "local": local,
            "summary": summary,
            "rows": [r.to_dict() for r in rows],
            "failures": failures,
        }, indent=2) + "\n")

    if args.md:
        args.md.parent.mkdir(parents=True, exist_ok=True)
        names = ["lccc", *oracles]
        lines = ["| function | source | " + " | ".join(names) +
                 " | best | LCCC/best |", "|---|---|" + "---:|" * (len(names) + 1) + "---:|"]
        key = (lambda r: r.lccc_runtime_over_best) if args.mode == "runtime" \
            else (lambda r: r.lccc_over_best)
        for row in sorted(rows, key=lambda r: -(key(r) or 0.0)):
            cells = []
            for name in names:
                if args.mode == "runtime":
                    value = row.runtime.get(name)
                    cells.append("-" if value is None else f"{value * 1e3:.1f}")
                else:
                    insns = row._insns(name)
                    cells.append(str(insns) if insns is not None else "!err")
            lines.append(f"| `{row.function}` | {row.source} | " +
                         " | ".join(cells) +
                         f" | {row.best_name} | {_fmt(key(row))} |")
        args.md.write_text("\n".join(lines) + "\n")

    if failures:
        for failure in failures:
            print(f"error: {failure}", file=sys.stderr)
        return 1

    if args.fail_on_behind is not None:
        tolerance = 1.0 + args.fail_on_behind / 100.0
        key = (lambda r: r.lccc_runtime_over_best) if args.mode == "runtime" \
            else (lambda r: r.lccc_over_best)
        if not summary["ranked"]:
            print("FAIL: no comparable LCCC measurements", file=sys.stderr)
            return 1
        offenders = [r for r in rows if key(r) and key(r) > tolerance]
        if offenders:
            print(f"\nFAIL: {len(offenders)} function(s) more than "
                  f"{args.fail_on_behind:g}% behind the best oracle:", file=sys.stderr)
            for row in offenders:
                print(f"  {row.function} ({row.source}) {key(row):.3f}", file=sys.stderr)
            return 1
        print(f"\nPASS: nothing more than {args.fail_on_behind:g}% behind the best oracle")
    return 0


if __name__ == "__main__":
    sys.exit(main())
