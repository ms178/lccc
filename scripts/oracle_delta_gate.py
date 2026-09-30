#!/usr/bin/env python3
"""Compiler-Explorer oracle delta gate (BACKLOG MS-01, the "CE delta half").

``ci-codegen-gate.py`` is the *structural* gate (local LCCC output only).  This
tool adds the *competitive* half: for every entry of
``tests/oracle/delta_corpus.json`` it compiles the function with local LCCC and
with GCC / Clang / ICX on Compiler Explorer (through ``scripts/godbolt.py``, so
the shared on-disk cache and the pinned-alias policy apply), extracts four
deterministic, hardware-independent metrics per compiler and

* writes one JSON report (CI artifact; every oracle's resolved id is recorded),
* **fails** when LCCC regresses against the committed baseline
  ``engineering/evidence/oracle_delta_baseline.json`` by more than the
  tolerance (default +5 % instructions, +2 stack refs, +1 GOT ref, +1 push),
* prints the per-function gap to the *best* oracle (ratio < 1.0 == LCCC wins).

Metrics (AT&T, function body only): ``insns`` (labels/directives excluded),
``stack`` (``(%rsp)``/``(%rbp)`` operands), ``got`` (``@GOTPCREL``), ``push``.

Network policy: an unreachable Compiler Explorer is a *skip* (exit 0) unless
``--strict``; a gate that cannot run must not block unrelated work, but the
skip is loud and recorded in the report.

    scripts/oracle_delta_gate.py                       # gate
    scripts/oracle_delta_gate.py --update-baseline     # refresh after a win
    scripts/oracle_delta_gate.py --only ra01_global_match_probe --report r.json
    scripts/oracle_delta_gate.py --self-test
"""
from __future__ import annotations

import argparse
import json
import os
import re
import sys
from pathlib import Path
from typing import Any, Callable

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
sys.path.insert(0, str(HERE))

CORPUS = ROOT / "tests/oracle/delta_corpus.json"
BASELINE = ROOT / "engineering/evidence/oracle_delta_baseline.json"

STACK_RE = re.compile(r"\(%r[sb]p\)|\(%r[sb]p,")
TOLERANCE = {"insns_pct": 5.0, "stack": 2, "got": 1, "push": 1}


def metrics(body: list[str], count_insns: Callable[[list[str]], int]) -> dict[str, int]:
    """Deterministic codegen metrics of one function body (AT&T)."""
    ops = [ln.strip() for ln in body]
    return {
        "insns": count_insns(body),
        "stack": sum(1 for ln in ops if STACK_RE.search(ln)),
        "got": sum(1 for ln in ops if "GOTPCREL" in ln),
        "push": sum(1 for ln in ops if re.match(r"push[lq]?\s", ln)),
    }


def evaluate(rows: list[dict[str, Any]], baseline: dict[str, Any],
             tol: dict[str, float]) -> tuple[list[str], list[str]]:
    """Return ``(failures, notes)`` for measured ``rows`` against ``baseline``.

    A row is ``{"id", "lccc": metrics, "oracles": {name: metrics}}``.  Entries
    absent from the baseline are notes (new corpus entry), never failures."""
    failures: list[str] = []
    notes: list[str] = []
    base_entries = baseline.get("entries", {})
    for row in rows:
        ident, lccc = row["id"], row["lccc"]
        best = min((m["insns"] for m in row["oracles"].values()), default=None)
        if best:
            notes.append(f"{ident}: lccc {lccc['insns']} insns, best oracle {best} "
                         f"(ratio {lccc['insns'] / best:.2f})")
        ref = base_entries.get(ident)
        if ref is None:
            notes.append(f"{ident}: no baseline yet (run --update-baseline)")
            continue
        limit = ref["insns"] * (1.0 + tol["insns_pct"] / 100.0)
        if lccc["insns"] > limit:
            failures.append(f"{ident}: insns {lccc['insns']} > baseline {ref['insns']} "
                            f"(+{tol['insns_pct']:.0f}% = {limit:.1f})")
        for key in ("stack", "got", "push"):
            if lccc[key] > ref[key] + tol[key]:
                failures.append(f"{ident}: {key} {lccc[key]} > baseline {ref[key]} "
                                f"(+{tol[key]:g})")
    return failures, notes


def measure(entries: list[dict[str, str]], flags: str, oracles: list[str],
            local: str) -> tuple[list[dict[str, Any]], list[str]]:
    """Compile every entry locally and on CE.  Returns ``(rows, errors)``."""
    import godbolt  # late: keeps --self-test and unit tests import-light

    rows: list[dict[str, Any]] = []
    errors: list[str] = []
    compilers = godbolt.list_compilers("c")
    for entry in entries:
        src = ROOT / entry["source"]
        fn = entry["function"]
        text = src.read_text()
        local_body = godbolt._function_body(godbolt._compile_local(local, src, flags), fn)
        if local_body is None:
            errors.append(f"{entry['id']}: function {fn} not emitted by local LCCC")
            continue
        row: dict[str, Any] = {"id": entry["id"], "function": fn, "source": entry["source"],
                               "lccc": metrics(local_body, godbolt._instruction_count),
                               "oracles": {}, "resolved": {}}
        for name in oracles:
            cid = godbolt.resolve_compiler(name)
            meta = godbolt.compiler_metadata(cid, compilers=compilers)
            result = godbolt.compile_on_godbolt(cid, text, flags, intel=False)
            body = godbolt._function_body(godbolt.assembly_lines(result), fn) if result else None
            if body is None:
                errors.append(f"{entry['id']}: oracle {name} produced no body for {fn}")
                continue
            row["oracles"][name] = metrics(body, godbolt._instruction_count)
            row["resolved"][name] = {"id": cid, "name": meta.get("name", "")}
        rows.append(row)
    return rows, errors


def self_test() -> int:
    count = lambda body: sum(1 for ln in body if ln.strip() and not ln.strip().endswith(":")
                             and not ln.strip().startswith("."))
    body = ["f:", ".cfi_startproc", "  pushq %rbx", "  movq 8(%rsp), %rax",
            "  movq x@GOTPCREL(%rip), %rcx", "  ret"]
    m = metrics(body, count)
    assert m == {"insns": 4, "stack": 1, "got": 1, "push": 1}, m
    base = {"entries": {"a": {"insns": 100, "stack": 2, "got": 0, "push": 1}}}
    ok = {"id": "a", "lccc": {"insns": 104, "stack": 4, "got": 1, "push": 2},
          "oracles": {"gcc": {"insns": 60}, "icx": {"insns": 50}}}
    bad = {"id": "a", "lccc": {"insns": 106, "stack": 5, "got": 2, "push": 3},
           "oracles": {"gcc": {"insns": 60}}}
    new = {"id": "b", "lccc": {"insns": 10, "stack": 0, "got": 0, "push": 0}, "oracles": {}}
    fails, notes = evaluate([ok], base, TOLERANCE)
    assert not fails, fails
    assert any("ratio 2.08" in n for n in notes), notes
    fails, _ = evaluate([bad], base, TOLERANCE)
    assert len(fails) == 4, fails
    fails, notes = evaluate([new], base, TOLERANCE)
    assert not fails and any("no baseline" in n for n in notes)
    print("oracle_delta_gate self-test OK")
    return 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=(__doc__ or "").split("\n\n")[0])
    ap.add_argument("--local", default=str(ROOT / "target/fastbuild/lccc"))
    ap.add_argument("--corpus", type=Path, default=CORPUS)
    ap.add_argument("--baseline", type=Path, default=BASELINE)
    ap.add_argument("--only", help="comma-separated entry ids")
    ap.add_argument("--report", type=Path, help="write the JSON report here")
    ap.add_argument("--update-baseline", action="store_true")
    ap.add_argument("--strict", action="store_true", help="treat CE outages as failures")
    ap.add_argument("--insns-pct", type=float, default=TOLERANCE["insns_pct"])
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args(argv)
    if args.self_test:
        return self_test()

    corpus = json.loads(args.corpus.read_text())
    entries = corpus["entries"]
    if args.only:
        wanted = set(args.only.split(","))
        entries = [e for e in entries if e["id"] in wanted]
    if not Path(args.local).exists():
        print(f"error: local compiler {args.local} not built", file=sys.stderr)
        return 2
    try:
        rows, errors = measure(entries, corpus["flags"], corpus["oracles"], args.local)
    except Exception as exc:  # noqa: BLE001 - network/CE outage classification
        print(f"oracle gate SKIPPED: {type(exc).__name__}: {exc}", file=sys.stderr)
        return 1 if args.strict else 0
    tol = dict(TOLERANCE, insns_pct=args.insns_pct)
    baseline = json.loads(args.baseline.read_text()) if args.baseline.exists() else {}
    failures, notes = evaluate(rows, baseline, tol)
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(
            {"schema": 1, "flags": corpus["flags"], "rows": rows, "errors": errors,
             "failures": failures}, indent=2) + "\n")
    for line in notes:
        print(line)
    for line in errors:
        print(f"error: {line}", file=sys.stderr)
    if args.update_baseline:
        merged = baseline.get("entries", {})
        merged.update({r["id"]: r["lccc"] for r in rows})
        tmp = args.baseline.with_suffix(f".tmp.{os.getpid()}")
        tmp.write_text(json.dumps({"schema": 1, "flags": corpus["flags"],
                                   "entries": dict(sorted(merged.items()))}, indent=2) + "\n")
        tmp.replace(args.baseline)
        print(f"baseline updated: {args.baseline}")
        return 0
    for line in failures:
        print(f"REGRESSION {line}", file=sys.stderr)
    return 1 if failures or (errors and args.strict) else 0


if __name__ == "__main__":
    sys.exit(main())
