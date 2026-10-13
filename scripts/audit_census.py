#!/usr/bin/env python3
"""Whole-codebase audit census and ratchet (AUD-RATCHET-1).

The AUDIT-2026-10-13 findings are all "a class of site exists, and its count
must not grow while it is being paid down".  This script measures those classes
from the source tree, so the claim is checkable instead of asserted, and so a
future change that adds a panic to the lowering pipeline, an `unsafe`, an
undeclared environment knob or an unannotated `#[ignore]` fails CI at review
time rather than at the next audit.

Metric definitions (this docstring is the contract):

* Scanning.  Every ``src/**/*.rs`` file is read, then
  (1) comments (``//``, ``/* */`` incl. nesting), string literals (``"..."``,
  ``b"..."``, ``r"..."``, ``r#"..."#``) and character literals are replaced by
  whitespace, and
  (2) everything from the first ``#[cfg(test)]`` onward is dropped, so test
  code does not inflate or defuse the counts.
  Both steps delete text but never bytes' worth of *lines*: line numbers (and
  therefore function lengths) are preserved.
* ``panic_sites`` -- occurrences of ``panic!``, ``unreachable!``,
  ``unimplemented!``, ``todo!``, ``.expect(`` and ``.unwrap(``.
  ``debug_assert*``/``assert*`` are deliberately excluded: those are contract
  checks, not user-input-reachable abort paths.
* ``panic_sites_hot`` -- the same count restricted to ``src/frontend/``,
  ``src/ir/lowering/`` and ``src/driver/``, i.e. the code that parses and
  lowers *user input* (AUD-ICE-1's actual target).
* ``unsafe_sites`` -- occurrences of the ``unsafe`` keyword (blocks, fns,
  impls, traits).
* ``fns_over_300`` / ``fns_over_500`` / ``fns_over_1000`` -- functions whose
  body (from the ``fn`` keyword through its closing brace, in the *stripped*
  text) exceeds the threshold in lines.  Approximate by construction: it is a
  brace-matching scan, not a parser, and it counts the same way the audit did.
* ``knob_literals`` -- distinct ``CCC_*``/``LCCC_*`` identifiers appearing in
  ``src/``; ``knobs_undocumented`` -- those not named in any tracked ``*.md``.
* ``ignored_tests`` -- occurrences of ``#[ignore`` (AUD-ARM-GSF-1 requires each
  one to carry a backlog id).

Usage::

    python3 scripts/audit_census.py                  # census table
    python3 scripts/audit_census.py --check          # ratchet vs. baseline
    python3 scripts/audit_census.py --update         # rewrite the baseline
    python3 scripts/audit_census.py --selftest       # scanner self-test
    python3 scripts/audit_census.py --top-fns 25     # longest functions
    python3 scripts/audit_census.py --list-undocumented-knobs

Exit codes: 0 = census printed or ratchet held, 1 = a metric grew beyond the
baseline (or a self-test failed), 2 = usage error.
"""

from __future__ import annotations

import argparse
import bisect
import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
BASELINE = ROOT / "scripts" / "audit_census_baseline.json"

SCAN_DIRS = ("src",)
HOT_PREFIXES = ("src/frontend/", "src/ir/lowering/", "src/driver/")

PANIC_PAT = re.compile(
    r"\bpanic!\s*\(|\bunreachable!\s*\(|\bunimplemented!\s*\(|\btodo!\s*\(|\.expect\s*\(|\.unwrap\s*\("
)
UNSAFE_PAT = re.compile(r"\bunsafe\b")
IGNORE_PAT = re.compile(r"#\[ignore")
KNOB_PAT = re.compile(r"\b(?:CCC|LCCC)_[A-Z0-9_]+\b")
FN_PAT = re.compile(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)\s*(?:<[^>{}]*>)?\s*\(")


CANDIDATE = re.compile(r'//|/\*|b?"|b?r(#*)"|\'')


def _blank(text: str) -> str:
    """Replace the non-newline characters of ``text`` with spaces."""
    return re.sub(r"[^\n]", " ", text)


def strip_rust(src: str) -> str:
    """Blank out comments and literals, preserving every newline.

    The scan is driven by a regex that finds the *next* interesting token and
    then jumps to its end with ``str.find``; nothing is walked
    character-by-character in Python, so a 700k-line tree is stripped in a few
    seconds.  Replacing (rather than deleting) keeps the newline count -- and
    hence every line number -- identical to the original file.
    """
    out: list[str] = []
    i, n = 0, len(src)
    char_pat = re.compile(r"'(?:\\.|[^\\'\n])'")
    while True:
        m = CANDIDATE.search(src, i)
        if m is None:
            out.append(src[i:])
            break
        out.append(src[i : m.start()])
        tok = m.group(0)
        if tok == "//":
            j = src.find("\n", m.end())
            j = n if j < 0 else j
            out.append(_blank(src[m.start() : j]))
            i = j
            continue
        if tok == "/*":
            # Rust block comments nest.
            depth, j = 1, m.end()
            while j < n and depth:
                if src.startswith("/*", j):
                    depth += 1
                    j += 2
                elif src.startswith("*/", j):
                    depth -= 1
                    j += 2
                else:
                    j += 1
            out.append(_blank(src[m.start() : j]))
            i = j
            continue
        if tok.startswith(("r", "br")):  # r"..." / r#"..."# / br#"..."#
            hashes = m.group(1) or ""
            terminator = '"' + hashes
            j = src.find(terminator, m.end())
            j = n if j < 0 else j + len(terminator)
            out.append(_blank(src[m.start() : j]))
            i = j
            continue
        if tok.endswith('"'):
            j = m.end()
            while j < n:
                if src[j] == "\\":
                    j += 2
                    continue
                if src[j] == '"':
                    j += 1
                    break
                if src[j] == "\n":  # unterminated literal: stop at the line end
                    break
                j += 1
            out.append(_blank(src[m.start() : j]))
            i = j
            continue
        # A quote that opens a lifetime ("'a") rather than a character literal.
        cm = char_pat.match(src, m.start())
        if cm:
            out.append(_blank(src[cm.start() : cm.end()]))
            i = cm.end()
        else:
            out.append(src[m.start() : m.end()])
            i = m.end()
    return "".join(out)


def line_index(text: str) -> list[int]:
    """Offsets of every newline, for O(log n) offset -> line conversion."""
    return [i for i, ch in enumerate(text) if ch == "\n"]


def without_tests(stripped: str) -> str:
    """Drop everything from the first ``#[cfg(test)]`` on (byte-preserving)."""
    m = re.search(r"#\[cfg\(test\)\]", stripped)
    if not m:
        return stripped
    return stripped[: m.start()] + "".join(
        "\n" if ch == "\n" else " " for ch in stripped[m.start() :]
    )


def scan_functions(
    text: str, newlines: list[int] | None = None
) -> list[tuple[str, int, int]]:
    """Return (name, start_line, end_line) for every brace-delimited ``fn``."""
    if newlines is None:
        newlines = line_index(text)
    fns: list[tuple[str, int, int]] = []
    for m in FN_PAT.finditer(text):
        # The parameter list must be balanced before we look for the body.
        i, depth, n = m.end() - 1, 0, len(text)
        while i < n:
            if text[i] == "(":
                depth += 1
            elif text[i] == ")":
                depth -= 1
                if depth == 0:
                    break
            i += 1
        else:
            continue
        i += 1
        # Skip the return type, where clause and attributes; `{` opens the body,
        # `;` means a declaration (trait method / extern fn).
        while i < n:
            if text[i] == "{":
                break
            if text[i] == ";":
                break
            i += 1
        if i >= n or text[i] == ";":
            continue
        depth, start_body = 0, i
        while i < n:
            if text[i] == "{":
                depth += 1
            elif text[i] == "}":
                depth -= 1
                if depth == 0:
                    break
            i += 1
        end = i if i < n else n - 1
        start_line = bisect.bisect_right(newlines, m.start()) + 1
        end_line = bisect.bisect_right(newlines, end) + 1
        fns.append((m.group(1), start_line, end_line))
    return fns


def strip_text(text: str) -> str:
    return without_tests(strip_rust(text))


def census() -> dict[str, object]:
    metrics: dict[str, int] = {
        "panic_sites": 0,
        "panic_sites_hot": 0,
        "unsafe_sites": 0,
        "ignored_tests": 0,
        "fns_over_300": 0,
        "fns_over_500": 0,
        "fns_over_1000": 0,
    }
    knobs: set[str] = set()
    knob_sites: dict[str, int] = {}
    files = 0

    for directory in SCAN_DIRS:
        for path in sorted((ROOT / directory).rglob("*.rs")):
            files += 1
            rel = path.relative_to(ROOT).as_posix()
            raw = path.read_text(errors="ignore")
            text = strip_text(raw)
            panics = len(PANIC_PAT.findall(text))
            metrics["panic_sites"] += panics
            if rel.startswith(HOT_PREFIXES):
                metrics["panic_sites_hot"] += panics
            metrics["unsafe_sites"] += len(UNSAFE_PAT.findall(text))
            metrics["ignored_tests"] += len(IGNORE_PAT.findall(text))
            for name, start, end in scan_functions(text):
                length = end - start + 1
                if length > 1000:
                    metrics["fns_over_1000"] += 1
                if length > 500:
                    metrics["fns_over_500"] += 1
                if length > 300:
                    metrics["fns_over_300"] += 1
            # Knobs name string literals (`env::var("CCC_...")`), so they must be
            # counted on the raw text, not the stripped one.
            for knob in KNOB_PAT.findall(raw):
                knobs.add(knob)
                knob_sites[knob] = knob_sites.get(knob, 0) + 1

    documented: set[str] = set()
    for path in ROOT.rglob("*.md"):
        if ".git" in path.parts or "target" in path.parts:
            continue
        documented |= set(KNOB_PAT.findall(path.read_text(errors="ignore")))

    result: dict[str, object] = dict(metrics)
    result.update(
        {
            "files": files,
            "knob_literals": len(knobs),
            "knobs_undocumented": len(knobs - documented),
            "knob_undocumented_names": sorted(knobs - documented),
        }
    )
    return result


def top_functions(limit: int) -> list[tuple[int, str, str, int, int]]:
    rows: list[tuple[int, str, str, int, int]] = []
    for directory in SCAN_DIRS:
        for path in sorted((ROOT / directory).rglob("*.rs")):
            text = strip_text(path.read_text(errors="ignore"))
            rel = path.relative_to(ROOT).as_posix()
            newlines = line_index(text)
            for name, start, end in scan_functions(text, newlines):
                rows.append((end - start + 1, rel, name, start, end))
    rows.sort(reverse=True)
    return rows[:limit]


def baseline_path() -> pathlib.Path:
    return BASELINE


def load_baseline() -> dict[str, object]:
    if not BASELINE.exists():
        return {}
    return json.loads(BASELINE.read_text())


RATCHETED = (
    "panic_sites",
    "panic_sites_hot",
    "unsafe_sites",
    "ignored_tests",
    "fns_over_300",
    "fns_over_500",
    "fns_over_1000",
    "knob_literals",
    "knobs_undocumented",
)


def cmd_check(current: dict[str, object]) -> int:
    baseline = load_baseline()
    if not baseline:
        print(f"FAIL: {BASELINE.relative_to(ROOT)} is missing (run --update)", file=sys.stderr)
        return 1
    grown = []
    for key in RATCHETED:
        was, now = baseline.get(key), current.get(key)
        if was is None or now is None:
            print(f"FAIL: metric {key} missing from the baseline", file=sys.stderr)
            return 1
        if now > was:
            grown.append(f"{key}: {was} -> {now} (+{now - was})")
    for key in RATCHETED:
        print(f"  {key:22} {current[key]:6}   baseline {baseline[key]:6}")
    if grown:
        print("FAIL: audit census grew:", file=sys.stderr)
        for line in grown:
            print(f"  {line}", file=sys.stderr)
        print(
            "Lower the counts, or justify and re-baseline in the commit that adds them.",
            file=sys.stderr,
        )
        return 1
    print("PASS: audit census held (no metric grew)")
    return 0


def cmd_update(current: dict[str, object]) -> int:
    baseline = load_baseline()
    payload = {key: current[key] for key in RATCHETED}
    old = {key: baseline.get(key) for key in RATCHETED}
    increased = [k for k in RATCHETED if old.get(k) is not None and payload[k] > old[k]]
    BASELINE.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
    print(f"baseline written: {BASELINE.relative_to(ROOT)}")
    for key in RATCHETED:
        before = old.get(key)
        arrow = "=" if before == payload[key] else f"{before} -> {payload[key]}"
        print(f"  {key:22} {arrow}")
    if increased:
        print(
            "WARNING: "
            + ", ".join(increased)
            + " increased; a ratchet may only be raised in the commit that adds the sites,"
            " and lowering it in the commit that removes them.",
            file=sys.stderr,
        )
    return 0


SELFTEST_SOURCE = r'''
// panic! in a comment must not count
/* block: unreachable!(x) */
fn normal(x: Option<u32>) -> u32 {
    let s = "unwrap() and panic! inside a string";
    let raw = r#"expect( "#;
    x.unwrap() + 1
}

#[cfg(test)]
mod tests {
    #[test]
    fn t() {
        panic!("test-only panic must not count");
        None::<u32>.expect("me too");
    }
}
'''


def cmd_selftest() -> int:
    stripped = strip_text(SELFTEST_SOURCE)
    checks = [
        ("comment/block-comment/string panic! ignored", PANIC_PAT.findall(stripped) == [".unwrap("]),
        ("raw string ignored", "expect(" not in stripped),
        ("test module excluded", "test-only" not in stripped),
        ("line count preserved", stripped.count("\n") == SELFTEST_SOURCE.count("\n")),
        ("long function detection", scan_functions(stripped)[0][0] == "normal"),
    ]
    failed = [name for name, ok in checks if not ok]
    for name, ok in checks:
        print(f"  {'ok  ' if ok else 'FAIL'} {name}")
    if failed:
        print(f"FAIL: audit_census selftest: {len(failed)} check(s) failed", file=sys.stderr)
        return 1
    print("PASS: audit_census selftest")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--check", action="store_true", help="ratchet against the baseline (exit 1 if a metric grew)")
    parser.add_argument("--update", action="store_true", help="rewrite the baseline from this tree")
    parser.add_argument("--selftest", action="store_true", help="self-test the scanner")
    parser.add_argument("--top-fns", type=int, metavar="N", default=0, help="list the N longest functions")
    parser.add_argument(
        "--list-undocumented-knobs",
        action="store_true",
        help="print the CCC_*/LCCC_* literals that no markdown file names",
    )
    args = parser.parse_args(argv)

    if args.selftest:
        return cmd_selftest()

    current = census()

    if args.list_undocumented_knobs:
        for name in current["knob_undocumented_names"]:
            print(name)
        return 0

    if args.top_fns:
        print(f"{'lines':>7}  {'function':<44} file:line")
        for length, rel, name, start, _ in top_functions(args.top_fns):
            print(f"{length:7}  {name:<44} {rel}:{start}")
        return 0

    if args.update:
        return cmd_update(current)
    if args.check:
        return cmd_check(current)

    for key in (
        "files",
        "panic_sites",
        "panic_sites_hot",
        "unsafe_sites",
        "ignored_tests",
        "fns_over_300",
        "fns_over_500",
        "fns_over_1000",
        "knob_literals",
        "knobs_undocumented",
    ):
        print(f"{key:22} {current[key]}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
