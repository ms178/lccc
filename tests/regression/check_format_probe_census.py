#!/usr/bin/env python3
"""Census ratchet: no NEW `format!`-built string probes in backend production code.

THE DEFECT CLASS
----------------
A probe pattern assembled with `format!` and handed straight to a rejecting
string test --

    t[1].strip_prefix(&format!("{mn}{} ", if q { 'q' } else { 'l' }))   // per window!
    if s == format!("testl {rn}, {rn}") { .. }                          // per Cmp line!

-- heap-allocates the pattern on EVERY probe, including the overwhelming
majority that reject.  Inside a peephole window walk that multiplies by line
count and fixpoint iteration count.  DHAT on the 40-function mixed TU at -O1
measured 159,052 eight-byte blocks from exactly two such sites
(`match_preloaded_scan_copy` / `match_guarded_scan_select`), every one freed
within the probe that made it.

Every instance is expressible without the allocation:

  * finite pattern set        -> static spellings  (`"bsfl "`, `"bsfq "`, ...)
  * pattern loop-invariant    -> hoist one owned pattern out of the loop
  * concatenation probe       -> chained `strip_prefix`/`strip_suffix` + `==`
    (`strip_suffix(&format!(", {rn}"))` == `strip_suffix(rn)` then
    `strip_suffix(", ")` -- both mandatory, byte-for-byte the same acceptance)

WHY A CENSUS AND NOT AN OUTRIGHT BAN
------------------------------------
After the measured-hot sites were converted, DHAT shows none of the remaining
sites in the top-20 allocation census: they sit behind an earlier gate (a scan
already matched, a mnemonic already compared equal) and run orders of
magnitude less often.  Converting ~70 cold sites by hand in one change would
maximise the chance of one subtle acceptance change (a miscompile) for no
measurable win.  The census therefore PINS the per-file count: additions fail
this gate, deletions are rewarded by ratcheting the baseline down.  New code
must use the allocation-free forms from the start.

Test modules are excluded (a trailing `#[cfg(test)] mod` is skipped): a test
asserting on a formatted string is not a per-line compile-time cost.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[2]
BACKEND = ROOT / "src/backend"
BASELINE = Path(__file__).with_name("format_probe_census_baseline.json")

# A probe is a rejecting string test whose pattern argument is a fresh
# `format!` allocation, or an equality against one.
PROBE = re.compile(
    r"\.(?:strip_prefix|strip_suffix|starts_with|ends_with|contains)\(\s*&format!\("
    r"|[=!]= format!\("
    r"|format!\([^)]*\) [=!]="
)
CFG_TEST = re.compile(r"^\s*#\[cfg\(test\)\]\s*$")


def census(path: Path) -> int:
    """Count probe sites in production code (test modules excluded)."""
    count = 0
    in_test_mod = False
    pending_cfg_test = False
    for raw in path.read_text().split("\n"):
        stripped = raw.strip()
        if in_test_mod:
            # Test modules in this codebase close at column 0; a top-level
            # `}` ends the region. Nested mods inside keep us in-test.
            if raw.startswith("}"):
                in_test_mod = False
            continue
        if pending_cfg_test:
            pending_cfg_test = False
            if stripped.startswith(("mod ", "pub mod ", "pub(crate) mod ", "pub(super) mod ")):
                in_test_mod = True
                continue
        if CFG_TEST.match(raw):
            pending_cfg_test = True
            continue
        code = raw.split("//", 1)[0]
        count += len(PROBE.findall(code))
    return count


def collect() -> dict[str, int]:
    out: dict[str, int] = {}
    for path in sorted(BACKEND.rglob("*.rs")):
        n = census(path)
        if n:
            out[str(path.relative_to(ROOT))] = n
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--write-baseline", action="store_true",
                    help="regenerate the pinned census (use ONLY when sites were removed)")
    args = ap.parse_args()
    current = collect()
    if args.write_baseline:
        BASELINE.write_text(json.dumps(current, indent=2, sort_keys=True) + "\n")
        print(f"wrote {BASELINE.name}: {sum(current.values())} site(s) in {len(current)} file(s)")
        return 0
    if not BASELINE.exists():
        print(f"FAIL: missing baseline {BASELINE.name}; run --write-baseline", file=sys.stderr)
        return 2
    baseline: dict[str, int] = json.loads(BASELINE.read_text())
    failed = 0
    for rel, n in sorted(current.items()):
        pinned = baseline.get(rel, 0)
        if n > pinned:
            print(f"FAIL: {rel}: {n} format!-built probe(s), baseline pins {pinned}. "
                  f"Use a static spelling, a hoisted pattern, or chained "
                  f"strip_prefix/strip_suffix (see this script's doc comment).")
            failed += 1
    shrank = {rel: (baseline[rel], current.get(rel, 0))
              for rel in baseline if current.get(rel, 0) < baseline[rel]}
    if failed:
        print(f"check_format_probe_census: {failed} file(s) above baseline")
        return 1
    note = ""
    if shrank:
        note = (f"; {len(shrank)} file(s) below baseline -- ratchet down with "
                f"--write-baseline")
    print(f"check_format_probe_census: PASS "
          f"({sum(current.values())} pinned site(s) in {len(current)} file(s){note})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
