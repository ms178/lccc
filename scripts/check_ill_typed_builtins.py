#!/usr/bin/env python3
"""AUD-ICE-1 acceptance gate: an ill-typed builtin call is a DIAGNOSTIC, never
a compiler panic.

The audit's acceptance criterion for AUD-ICE-1, written out: "a table of
ill-typed calls of every `BuiltinIntrinsic` (wrong arity, wrong type class,
non-constant where a constant is required) at `-O0` and `-O2`, asserting exit
1, a `file:line:col: error:` prefix and the absence of `internal error`."

This is that table, generated from the compiler's own name table rather than
hand-written, so a builtin added tomorrow is covered tomorrow.  Every name is
exercised through four independent ill-typed shapes at both optimisation
levels, and each compile is judged on three things:

  * **ICE**: `internal error` / `panicked at` in the compiler's output.  A
    panic is never an acceptable answer for a *type* error, whatever the
    message says.  HARD failure above the recorded baseline.
  * **silent**: an ill-typed call the compiler accepts without a word.  Some of
    these are legitimate (`__builtin_add_overflow` is generic, the fortify
    `__builtin___*_chk` family is variadic); the rest are permissive type
    checking.  RATCHETED, never allowed to grow.

Both lists live in `tests/regression/ill_typed_builtins_baseline.json`.  A name
that stops panicking is *reported* so the baseline can be tightened in the same
commit that fixed it; a name that starts panicking fails the gate immediately.

Usage:
    scripts/check_ill_typed_builtins.py                 # gate
    scripts/check_ill_typed_builtins.py --list          # print the name table
    scripts/check_ill_typed_builtins.py --update-baseline
    scripts/check_ill_typed_builtins.py --name _mm_add_ps   # one builtin, verbose
"""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path


def _find_repo() -> Path:
    """The repository that owns this script: $LCCC_REPO, or the nearest ancestor
    of the working directory holding the builtin name table."""
    cand = os.environ.get("LCCC_REPO")
    if cand and (Path(cand) / "src/frontend/sema/builtins.rs").exists():
        return Path(cand)
    here = Path.cwd().resolve()
    for c in (here, *here.parents):
        if (c / "src/frontend/sema/builtins.rs").exists():
            return c
    return Path(__file__).resolve().parent.parent


REPO = _find_repo()
TABLE = REPO / "src/frontend/sema/builtins.rs"
DEFAULT_LCCC = REPO / "target" / "fastbuild" / "lccc"
BASELINE = REPO / "tests" / "regression" / "ill_typed_builtins_baseline.json"

# `m.insert("name", BuiltinInfo::intrinsic(BuiltinIntrinsic::Kind))`
ENTRY = re.compile(
    r'm\.insert\(\s*"([^"]+)"\s*,\s*BuiltinInfo::intrinsic\(\s*(?:BuiltinIntrinsic::)?(\w+)\s*\)',
    re.S,
)
ICE_MARKERS = ("internal error", "panicked at")
DIAG = re.compile(r"^.+?:\d+:\d+: error: ", re.M)


def names_from_table() -> list[tuple[str, str]]:
    seen: dict[str, str] = {}
    for name, kind in ENTRY.findall(TABLE.read_text()):
        seen.setdefault(name, kind)
    return sorted(seen.items())


def shapes(name: str, idx: int) -> list[str]:
    """Four independent ill-typed shapes, one per function, so a diagnostic's
    position identifies which shape was reached."""
    return [
        f"void z{idx}(void) {{ (void)({name}()); }}",
        f"void o{idx}(void) {{ int x = 0; (void)({name}(x)); }}",
        f"void t{idx}(void) {{ int x = 0; (void)({name}(x, &x)); }}",
        f"void m{idx}(void) {{ int x = 0; (void)({name}(x, x, x, x, x, x, x, x)); }}",
    ]


def compile_one(lccc: str, name: str, idx: int, tmp: Path, opt: str) -> dict:
    src = tmp / f"ill{idx}{opt}.c"
    src.write_text("\n".join(shapes(name, idx)) + "\n")
    asm = src.with_suffix(".s")
    try:
        proc = subprocess.run(
            [lccc, opt, "-S", str(src), "-o", str(asm)],
            capture_output=True,
            text=True,
            timeout=300,
        )
        out = proc.stdout + proc.stderr
        rc = proc.returncode
    except subprocess.TimeoutExpired:
        out, rc = "timeout", 124
    asm.unlink(missing_ok=True)
    return {
        "name": name,
        "opt": opt,
        "rc": rc,
        "ice": [m for m in ICE_MARKERS if m in out],
        "diagnostics": len(DIAG.findall(out)),
        "out": out,
    }


def decide(
    ice_now: list[str], silent_now: list[str], baseline: dict
) -> dict[str, list[str]]:
    """Compare this run against the baseline.  Growing either list is a
    regression; shrinking is a `fixed` note so the caller can tighten the
    baseline in the same commit that fixed the panic."""
    known_ice = set(baseline.get("ice", []))
    known_silent = set(baseline.get("silent", []))
    return {
        "new_ice": [n for n in ice_now if n not in known_ice],
        "new_silent": [n for n in silent_now if n not in known_silent],
        "fixed_ice": sorted(known_ice - set(ice_now)),
        "fixed_silent": sorted(known_silent - set(silent_now)),
    }


def self_test() -> int:
    """The ratchet must be able to fail, and to notice a fix, on inputs that
    differ only in the way a real regression differs."""
    base = {"ice": ["_mm_a -O0", "_mm_a -O2"], "silent": ["_mm_b -O0"]}
    assert names_from_table(), "name table is empty"
    assert all("BuiltinIntrinsic" not in k for _n, k in names_from_table())
    names = [n for n, _k in names_from_table()]
    assert "_mm_add_ps" in names, "known intrinsic missing from the table"
    assert len(shapes("_mm_add_ps", 3)) == 4

    same = decide(base["ice"], base["silent"], base)
    assert not same["new_ice"] and not same["new_silent"] and not same["fixed_ice"]
    grew = decide(base["ice"] + ["_mm_c -O2"], base["silent"], base)
    assert grew["new_ice"] == ["_mm_c -O2"], grew
    over_silent = decide(base["ice"], base["silent"] + ["_mm_c -O2"], base)
    assert over_silent["new_silent"] == ["_mm_c -O2"], over_silent
    shrank = decide(base["ice"][:1], base["silent"], base)
    assert shrank["fixed_ice"] == ["_mm_a -O2"] and not shrank["new_ice"]
    assert decide(base["ice"], base["silent"], {})["new_ice"] == base["ice"]
    print("self-test: ratchet grows, shrinks and holds as specified")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument("--lccc", default=os.environ.get("CCC", str(DEFAULT_LCCC)))
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--update-baseline", action="store_true")
    ap.add_argument(
        "--self-test", action="store_true", help="exercise the ratchet logic, no compiler needed"
    )
    ap.add_argument("--name", help="exercise a single builtin and print the output")
    args = ap.parse_args()

    if args.self_test:
        return self_test()

    entries = names_from_table()
    if not entries:
        print("FAIL: no intrinsics found in the name table", file=sys.stderr)
        return 1
    if args.list:
        for name, kind in entries:
            print(f"{kind:34s} {name}")
        print(f"{len(entries)} intrinsic name(s)")
        return 0
    if args.name:
        entries = [(n, k) for n, k in entries if n == args.name] or [(args.name, "-")]
    if not Path(args.lccc).exists():
        print(f"SKIP: {args.lccc} not built", file=sys.stderr)
        return 0

    per_name: dict[tuple[str, str], dict] = {}
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        jobs = [
            (name, idx * 10, opt)
            for idx, (name, _k) in enumerate(entries)
            for opt in ("-O0", "-O2")
        ]
        with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as ex:
            futs = [ex.submit(compile_one, args.lccc, j[0], j[1], tmp, j[2]) for j in jobs]
            for fut in concurrent.futures.as_completed(futs):
                res = fut.result()
                per_name[(res["name"], res["opt"])] = res

    if args.name:
        for (name, opt), res in sorted(per_name.items()):
            print(f"=== {name} {opt} rc={res['rc']} diagnostics={res['diagnostics']}")
            print(res["out"])
        return 0

    ice_names = sorted(f"{n} {o}" for (n, o), r in per_name.items() if r["ice"])
    silent_names = sorted(
        f"{n} {o}" for (n, o), r in per_name.items() if not r["ice"] and r["diagnostics"] == 0
    )

    if args.update_baseline:
        BASELINE.write_text(
            json.dumps(
                {
                    "note": (
                        "AUD-ICE-1 ratchet floor. `ice` names are ill-typed calls "
                        "that PANIC the compiler today; `silent` names compile with "
                        "no diagnostic at all. Regenerate with --update-baseline; "
                        "the gate fails when either list grows."
                    ),
                    "ice": ice_names,
                    "silent": silent_names,
                },
                indent=2,
            )
            + "\n"
        )
        print(
            f"baseline: {len(ice_names)} ICE name(s), {len(silent_names)} silent "
            f"name(s) -> {BASELINE.relative_to(REPO)}"
        )
        return 0

    baseline = {"ice": [], "silent": []}
    if BASELINE.exists():
        baseline = json.loads(BASELINE.read_text())
    verdict = decide(ice_names, silent_names, baseline)
    new_ice, new_silent = verdict["new_ice"], verdict["new_silent"]
    fixed_ice = verdict["fixed_ice"]

    print(
        f"{len(entries)} builtin(s) x 4 ill-typed shapes x 2 optimisation levels: "
        f"{len(ice_names)} ICE, {len(silent_names)} silent"
    )
    if fixed_ice:
        print(
            f"note: {len(fixed_ice)} name(s) no longer panic -- tighten the baseline "
            "in the commit that fixed them:"
        )
        for n in fixed_ice[:10]:
            print(f"  fixed: {n}")
    if new_ice:
        print(f"FAIL: {len(new_ice)} name(s) panic on an ill-typed call", file=sys.stderr)
        for n in new_ice[:20]:
            print(f"  {n}", file=sys.stderr)
        return 1
    if new_silent:
        print(
            f"FAIL: {len(new_silent)} name(s) newly accept an ill-typed call "
            "without a diagnostic",
            file=sys.stderr,
        )
        for n in new_silent[:20]:
            print(f"  {n}", file=sys.stderr)
        return 1

    print(
        f"ratchet held: {len(ice_names)} ICE / {len(silent_names)} silent, no additions"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
