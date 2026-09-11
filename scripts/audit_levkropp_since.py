#!/usr/bin/env python3
"""Audit levkropp/lccc commits against the current ms178/lccc tree.

The script is intentionally offline/reproducible: it consumes a locally fetched
`levkropp/main` ref and scans the checked-out ms178 tree for concrete adoption
markers.  It is not a substitute for semantic review, but it gives future agents
an exact, rerunnable inventory of levkropp commits since a date, their touched
paths, and whether the current tree contains the feature/file names that make a
change plausibly adopted.

Usage:
  git fetch levkropp main --shallow-since=2026-08-19T00:00:00Z
  scripts/audit_levkropp_since.py --since 2026-08-19 --out engineering/evidence/levkropp-audit.md
"""
from __future__ import annotations

import argparse
import subprocess
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class Rule:
    needle: str
    markers: tuple[str, ...]
    category: str


RULES: tuple[Rule, ...] = (
    Rule("Backedge PRE", ("src/passes/backedge_pre.rs", "backedge_pre::run", "bepre"), "adopted-this-session/int-only"),
    Rule("pointer +=/-=", ("pointer +=", "rhs_wide", "pointer_compound_assign_signed_index"), "adopted-this-session"),
    Rule("volatile forwarding guard", ("volatile / semantic_volatile allocas are never tracked", "if *volatile"), "already-adopted"),
    Rule("quadratic", ("src/passes/quadratic_sr.rs", "quadratic_sr::run"), "already-adopted"),
    Rule("redundant-load", ("src/passes/redundant_loads.rs", "redundant_loads::run"), "already-adopted"),
    Rule("plain-copy loops", ("analyze_copy_scale_add_loop", "copy/scale/add", "vectorize"), "partially/adopted"),
    Rule("GlobalAddr CSE", ("global_addr_cse", "must_materialize_global_addr"), "already-adopted"),
    Rule("aggregate temp", ("aggregate_copy_forward", "param_slot_roots"), "already-adopted"),
    Rule("cross-kind disjointness", ("forms_disjoint", "cross", "march"), "already-adopted"),
    Rule("LICM use-before-def", ("defined_before", "LICM", "use-before-def"), "needs-manual-check"),
    Rule("mem2reg span", ("source_spans", "mem2reg", "span"), "needs-manual-check"),
    Rule("sqlite workload", ("sqlite", "workload"), "needs-manual-check"),
)


def run(args: list[str]) -> str:
    return subprocess.check_output(args, text=True, stderr=subprocess.DEVNULL)


def file_list(commit: str) -> list[str]:
    out = run(["git", "show", "--name-only", "--pretty=format:", commit])
    return [ln for ln in out.splitlines() if ln.strip()]


def tree_contains(marker: str, root: Path) -> bool:
    p = root / marker
    if p.exists():
        return True
    try:
        subprocess.check_output(["grep", "-R", "-F", "--", marker, "src", "tests"], cwd=root, stderr=subprocess.DEVNULL)
        return True
    except subprocess.CalledProcessError:
        return False


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--since", default="2026-08-19")
    ap.add_argument("--ref", default="levkropp/main")
    ap.add_argument("--out", default="-")
    ns = ap.parse_args()
    root = Path.cwd()
    log = run([
        "git", "log", "--reverse", f"--since={ns.since}T00:00:00Z",
        "--date=short", "--pretty=format:%h%x09%ad%x09%s", ns.ref,
    ])
    rows = []
    for line in log.splitlines():
        sha, date, subject = line.split("\t", 2)
        files = file_list(sha)
        matched = [r for r in RULES if r.needle.lower() in subject.lower()]
        if matched:
            evidence = []
            cats = []
            for r in matched:
                hits = [m for m in r.markers if tree_contains(m, root)]
                evidence.append("; ".join(hits) if hits else "no marker")
                cats.append(r.category if hits else "not-found/manual")
            status = ", ".join(dict.fromkeys(cats))
            ev = " | ".join(evidence)
        else:
            status = "manual-review"
            ev = "topic-specific / see semantic notes"
        rows.append((sha, date, subject, status, ev, ", ".join(files[:5]) + (" ..." if len(files) > 5 else "")))

    md = [
        "# levkropp/lccc commit audit inventory", "",
        f"Ref: `{ns.ref}`", f"Since: `{ns.since}`", "",
        "| Commit | Date | Subject | Adoption status | Evidence | Touched paths |",
        "|---|---|---|---|---|---|",
    ]
    for sha, date, subj, status, ev, files in rows:
        def esc(s: str) -> str:
            return s.replace("|", "\\|")
        md.append(f"| `{sha}` | {date} | {esc(subj)} | {esc(status)} | {esc(ev)} | {esc(files)} |")
    md.append("")
    text = "\n".join(md)
    if ns.out == "-":
        print(text)
    else:
        Path(ns.out).write_text(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
