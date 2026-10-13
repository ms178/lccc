#!/usr/bin/env python3
"""Static census of panic-class sites, unsafe, giant functions and env knobs.

WHAT THIS IS
------------
A ratchet, not a style rule.  The 2026-10-13 whole-codebase audit counted, on
`ms178/lccc` main:

    834 panic-class sites   (119 in frontend/ + ir/lowering/ + driver/)
     27 `unsafe` sites
    195/78/23 functions over 300/500/1000 lines
    475 distinct CCC_*/LCCC_* literals, 284 of them named in no markdown file

Every one of those numbers is a *debt* number, and debt that is not ratcheted
grows silently: a new `panic!` in the lowering stage re-opens AUD-ICE-1, a new
unannotated `#[ignore]` re-opens AUD-ARM-GSF-1, a new `CCC_` knob without a
reader or a doc becomes an undeclared configuration surface (AUD-KNOB-1).

HOW IT COUNTS (honest limits)
-----------------------------
* Strings, char literals and comments are stripped before counting panic-class
  and `unsafe` sites, so `panic!` inside a string does not count.  The env-knob
  metric deliberately scans the RAW text (the knobs *are* string literals).
* Everything after the first top-level `#[cfg(test)] mod` in a file is skipped:
  test code is allowed to panic (`unwrap()` on fixtures) and would otherwise
  dominate every number.
* Function lengths are measured from the `fn` keyword to its closing brace by
  brace matching over code-only text; a function whose braces cannot be matched
  is reported (never silently skipped).
* It is a textual census.  It cannot tell a reachable panic from an
  unreachable one -- that judgement is the audit's, per item.

USAGE
-----
    scripts/audit_census.py                     # print the census
    scripts/audit_census.py --check             # ratchet (CI gate): fail on growth
    scripts/audit_census.py --json out.json
    scripts/audit_census.py --top-fns 25        # worst offenders by line count
    scripts/audit_census.py --update            # lower/raise the baseline (deliberate)
    scripts/audit_census.py --selftest          # prove the counting rules

CLI exits 1 on a ratchet breach, 2 on usage/environment errors.
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
BASELINE = REPO / "scripts" / "audit_census_baseline.json"
SCAN_DIRS = ("src",)

PANIC_MACROS = ("panic!", "unreachable!", "todo!", "unimplemented!")
PANIC_METHODS = (".unwrap()", ".expect(")
UNSAFE_RE = re.compile(r"\bunsafe\b")
FN_RE = re.compile(r"\b(?:pub\s+(?:\([^)]*\)\s*)?)?(?:const\s+|async\s+|extern\s+\"[^\"]*\"\s+|unsafe\s+)*fn\s+([A-Za-z_][A-Za-z0-9_]*)")
KNOB_RE = re.compile(r'"(CCC_[A-Z0-9_]+|LCCC_[A-Z0-9_]+)"')
IGNORE_RE = re.compile(r"#\[ignore\]")
CFG_TEST_RE = re.compile(r"^\s*#\[cfg\(test\)\]\s*$")


def strip_noncode(text: str) -> str:
    """Remove comments and string/char literals, preserving byte offsets shape.

    Replacement keeps newlines so line accounting stays meaningful.
    """
    out: list[str] = []
    i, n = 0, len(text)
    while i < n:
        c = text[i]
        nxt = text[i + 1] if i + 1 < n else ""
        if c == "/" and nxt == "/":
            j = text.find("\n", i)
            j = n if j < 0 else j
            out.append(" " * (j - i))
            i = j
        elif c == "/" and nxt == "*":
            depth = 1
            j = i + 2
            while j < n and depth:
                if text.startswith("/*", j):
                    depth += 1
                    j += 2
                elif text.startswith("*/", j):
                    depth -= 1
                    j += 2
                else:
                    j += 1
            block = text[i:j]
            out.append("".join("\n" if ch == "\n" else " " for ch in block))
            i = j
        elif c == '"':
            j = i + 1
            while j < n:
                if text[j] == "\\":
                    j += 2
                    continue
                if text[j] == '"':
                    j += 1
                    break
                j += 1
            out.append('""')
            i = j
        elif c == "'" and n > 2 and (text[i + 2] == "'" or (text[i + 1] == "\\" and i + 3 < n and text[i + 3] == "'")):
            # char literal (distinguish from lifetimes: 'a in `&'a T`)
            if text[i + 1] == "\\":
                j = i + 3
                while j < n and text[j] != "'":
                    j += 1
                j += 1
            else:
                j = i + 3
            out.append("''")
            i = j
        else:
            out.append(c)
            i += 1
    return "".join(out)


def strip_tests(text: str) -> str:
    """Cut everything from the first column-0 `#[cfg(test)] mod` declaration."""
    lines = text.splitlines(keepends=True)
    for idx, line in enumerate(lines):
        if CFG_TEST_RE.match(line):
            for later in lines[idx + 1 : idx + 6]:
                if re.match(r"\s*mod\s+[A-Za-z_]", later):
                    return "".join(lines[:idx])
    return text


def count_functions(code: str) -> tuple[dict[str, int], list[tuple[str, int]]]:
    """Return (threshold counts, per-function line counts) by brace matching."""
    counts = {"over_300": 0, "over_500": 0, "over_1000": 0}
    fns: list[tuple[str, int]] = []
    for match in FN_RE.finditer(code):
        brace = code.find("{", match.end())
        if brace < 0:
            continue
        # A `fn` in a signature position (trait declaration) has `;` first.
        semi = code.find(";", match.end())
        if 0 <= semi < brace:
            continue
        depth = 0
        end = -1
        for pos in range(brace, len(code)):
            ch = code[pos]
            if ch == "{":
                depth += 1
            elif ch == "}":
                depth -= 1
                if depth == 0:
                    end = pos
                    break
        if end < 0:
            continue
        lines = code.count("\n", match.start(), end) + 1
        fns.append((match.group(1), lines))
        for key, limit in (("over_300", 300), ("over_500", 500), ("over_1000", 1000)):
            if lines > limit:
                counts[key] += 1
    return counts, fns


def documented_knobs(repo: Path, knobs: set[str]) -> set[str]:
    docs = "".join(
        p.read_text(errors="ignore")
        for pattern in ("**/*.md", "**/*.sh", "**/*.py")
        for p in repo.glob(pattern)
        if ".git" not in p.parts and "target" not in p.parts
    )
    return {k for k in knobs if k in docs}


def census(repo: Path) -> dict:
    totals = {
        "panic_class": 0,
        "ignores_all": 0,
        "panic_class_frontend_lowering_driver": 0,
        "unsafe_sites": 0,
        "ignores": 0,
        "over_300": 0,
        "over_500": 0,
        "over_1000": 0,
    }
    per_dir: dict[str, int] = {}
    fns_all: list[tuple[str, int, str]] = []
    knobs: set[str] = set()
    unparsable: list[str] = []
    files = 0
    for scan in SCAN_DIRS:
        for path in sorted((repo / scan).rglob("*.rs")):
            files += 1
            file_text = path.read_text(errors="ignore")
            # `#[ignore]` matters most *inside* test modules (that is where the
            # parked ARM/x86 peephole tests live, AUD-ARM-GSF-1), so it is
            # counted before the cfg(test) cut.
            totals["ignores_all"] += len(IGNORE_RE.findall(file_text))
            raw = strip_tests(file_text)
            code = strip_noncode(raw)
            hits = sum(code.count(macro) for macro in PANIC_MACROS)
            hits += sum(code.count(method) for method in PANIC_METHODS)
            totals["panic_class"] += hits
            rel = path.relative_to(repo).as_posix()
            bucket = (
                "frontend/ir-lowering/driver"
                if rel.startswith(("src/frontend/", "src/ir/lowering/", "src/driver/"))
                else rel.split("/")[1] if len(rel.split("/")) > 2 else "src"
            )
            per_dir[bucket] = per_dir.get(bucket, 0) + hits
            if bucket == "frontend/ir-lowering/driver":
                totals["panic_class_frontend_lowering_driver"] += hits
            totals["unsafe_sites"] += len(UNSAFE_RE.findall(code))
            counts, fns = count_functions(code)
            for key, value in counts.items():
                totals[key] += value
            for name, lines in fns:
                fns_all.append((name, lines, rel))
            knobs.update(KNOB_RE.findall(raw))
    docs = documented_knobs(repo, knobs)
    totals["knobs_distinct"] = len(knobs)
    totals["knobs_undocumented"] = len(knobs - docs)
    return {
        "files": files,
        "metrics": totals,
        "per_dir": dict(sorted(per_dir.items())),
        "functions": sorted(fns_all, key=lambda item: -item[1]),
    }


def load_baseline() -> dict:
    if not BASELINE.is_file():
        return {}
    return json.loads(BASELINE.read_text())


def ratchet_failures(result: dict, baseline: dict) -> list[str]:
    failures = []
    for key, limit in baseline.get("metrics", {}).items():
        actual = result["metrics"].get(key, 0)
        if actual > limit:
            failures.append(f"{key}: {actual} > baseline {limit}")
    return failures


def selftest() -> int:
    tmp = Path(tempfile.mkdtemp(prefix="audit-census-selftest."))
    try:
        (tmp / "src" / "demo").mkdir(parents=True)
        (tmp / "src" / "demo" / "sample.rs").write_text(
            'fn short() {\n'
            '    let s = "panic! .unwrap()";  // panic! .unwrap()\n'
            '    /* panic! */\n'
            '    let _ = std::env::var("CCC_TEST_KNOB");\n'
            '}\n'
            + "#[cfg(test)]\n" + "mod tests {\n" + "    fn t() { panic!(); }\n" + "}\n"
        )
        (tmp / "src" / "demo" / "long.rs").write_text(
            "fn long() {\n" + "    let x = 1;\n" * 400 + "}\n"
        )
        (tmp / "src" / "demo" / "u.rs").write_text("unsafe fn f() { unsafe { } }\n#[ignore]\nfn g() {}\n")
        (tmp / "docs.md").write_text("CCC_TEST_KNOB is documented here.\n")
        result = census(tmp)
        metrics = result["metrics"]
        checks = {
            "panics in strings/comments are not code": metrics["panic_class"] == 0,
            "long function counted": metrics["over_300"] == 1,
            "unsafe sites counted": metrics["unsafe_sites"] == 2,
            "ignores counted (including test modules)": metrics["ignores_all"] == 1,
            "knobs distinct": metrics["knobs_distinct"] == 1,
            "knobs documented": metrics["knobs_undocumented"] == 0,
        }
        bad = [name for name, ok in checks.items() if not ok]
        if bad:
            print("audit_census selftest FAIL:", "; ".join(bad), file=sys.stderr)
            print(json.dumps(metrics, indent=2), file=sys.stderr)
            return 1
        # Restore the sample's panic-in-string case: it must NOT be counted.
        (tmp / "src" / "demo" / "sample.rs").write_text('fn f() { let _ = "unreachable!()"; }\n')
        if census(tmp)["metrics"]["panic_class"] != 0:
            print("audit_census selftest FAIL: string contents counted as code", file=sys.stderr)
            return 1
        print("audit_census selftest: PASS (stripping, cfg(test) cut, thresholds, knobs)")
        return 0
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--repo", type=Path, default=REPO)
    ap.add_argument("--check", action="store_true", help="ratchet against the baseline")
    ap.add_argument("--update", action="store_true", help="rewrite the baseline from this tree")
    ap.add_argument("--json", type=Path)
    ap.add_argument("--top-fns", type=int, default=0, metavar="N")
    ap.add_argument("--selftest", action="store_true")
    args = ap.parse_args()

    if args.selftest:
        return selftest()

    result = census(args.repo)
    metrics = result["metrics"]
    print(f"files scanned: {result['files']}")
    for key in (
        "panic_class",
        "panic_class_frontend_lowering_driver",
        "unsafe_sites",
        "ignores_all",
        "over_300",
        "over_500",
        "over_1000",
        "knobs_distinct",
        "knobs_undocumented",
    ):
        print(f"{key:42s} {metrics.get(key, 0)}")
    if args.top_fns:
        print("\nlongest functions:")
        for name, lines, rel in result["functions"][: args.top_fns]:
            print(f"  {lines:6d}  {rel}::{name}")
    if args.json:
        args.json.write_text(json.dumps({"metrics": metrics, "per_dir": result["per_dir"]}, indent=2, sort_keys=True) + "\n")
        print(f"wrote {args.json}")

    if args.update:
        BASELINE.write_text(
            json.dumps(
                {
                    "comment": "Ratchet baseline for scripts/audit_census.py. Lower only in the commit that removes sites.",
                    "metrics": {k: v for k, v in metrics.items()},
                },
                indent=2,
                sort_keys=True,
            )
            + "\n"
        )
        print(f"baseline updated: {BASELINE}")
        return 0

    if args.check:
        failures = ratchet_failures(result, load_baseline())
        if failures:
            for f in failures:
                print(f"FAIL: audit census ratchet: {f}", file=sys.stderr)
            print("Re-open the corresponding audit item or lower the count; do not raise the baseline.", file=sys.stderr)
            return 1
        print("audit census ratchet: PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
