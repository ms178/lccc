#!/usr/bin/env python3
"""pbt_ignore_ratchet.py — the PBT bug-witness ratchet.

The PBT suites carry `#[ignore = "documented bug: ..."]` witnesses: property
tests that assert the DESIRED behaviour and fail while the bug is open. An
ignore list with no ratchet is a graveyard with a growth direction -- nothing
fails when a 380th ignore is added, or when an ignore's bug is fixed but the
test is left ignored (dead weight that silently rots).

This gate pins both directions:

1. COUNT: the number of `#[ignore]` attributes under tests/pbt must not
   exceed the pinned maximum. Fixing bugs (and activating their witnesses)
   lowers the count; the pin ratchets down with it and never back up without
   a deliberate change to this file.

2. LIVENESS: every ignored test must still FAIL when run with `--ignored`.
   A witness that passes means its bug is fixed and the test is dead weight:
   activate it (remove the ignore) or delete it. The only exception is an
   ignore whose reason does not start with "documented bug" (sandbox skips,
   missing-tool skips) -- those are not witnesses and are exempt from the
   liveness check, though they still count toward the pin.

Usage:
    pbt_ignore_ratchet.py                 # run the gate
    pbt_ignore_ratchet.py --self-test     # verify the harness itself
    pbt_ignore_ratchet.py --update-pin    # rewrite PIN after a legit drop
"""
from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
PBT_DIR = REPO / "tests" / "pbt"
PIN_FILE = REPO / "tests" / "pbt" / "IGNORE_RATCHET_PIN"

# The PBT integration-test targets (see tests/pbt_*.rs).
TARGETS = [
    "pbt_arm_core",
    "pbt_arm_neon_a",
    "pbt_arm_neon_b",
    "pbt_arm_neon_c",
    "pbt_arm_load_store",
    "pbt_arm_fp_scalar",
    "pbt_arm_system",
    "pbt_common",
    "pbt_preprocessor",
]

IGNORE_RE = re.compile(r'#\[ignore\s*(?:=\s*"([^"]*)")?\s*\]')


def count_ignores() -> tuple[int, int]:
    """Returns (total, witnesses) over tests/pbt/**/*.rs."""
    total = witnesses = 0
    for path in sorted(PBT_DIR.rglob("*.rs")):
        for m in IGNORE_RE.finditer(path.read_text()):
            total += 1
            if m.group(1) and m.group(1).startswith("documented bug"):
                witnesses += 1
    return total, witnesses


def run_ignored() -> tuple[list[str], list[str]]:
    """Runs every PBT target with --ignored.

    Returns (passing_ignored, failing_ignored) test names. A passing ignored
    test is dead weight when it is a bug witness.
    """
    passing: list[str] = []
    failing: list[str] = []
    for target in TARGETS:
        # Default format: one "test NAME ... ok|FAILED" line per test.
        # cargo exits nonzero when any test fails -- the EXPECTED state for
        # witnesses -- so parse the listing regardless of the exit code.
        r = subprocess.run(
            ["cargo", "test", "--profile", "fastbuild", "--test", target,
             "--", "--ignored", "--test-threads", "2"],
            cwd=REPO, capture_output=True, text=True, timeout=1200,
        )
        out = r.stdout + r.stderr
        n_listed = 0
        for line in out.splitlines():
            m = re.match(r"test (\S+) \.\.\. (ok|FAILED)", line)
            if m:
                n_listed += 1
                (passing if m.group(2) == "ok" else failing).append(m.group(1))
        m_n = re.search(r"running (\d+) tests", out)
        n_tests = int(m_n.group(1)) if m_n else 0
        if n_tests != n_listed:
            # The harness output changed shape; fail closed rather than
            # silently approving an unparsed run.
            print(f"FAIL: could not parse {target} --ignored output "
                  f"({n_tests} announced, {n_listed} listed)")
            return None, None
    return passing, failing


def is_witness(name: str) -> bool:
    """A passing ignored test is only dead weight if it documents a bug.

    Sandbox/missing-tool skips (any ignore reason not starting with
    'documented bug') are legitimately green under --ignored.
    """
    for path in PBT_DIR.rglob("*.rs"):
        text = path.read_text()
        for m in IGNORE_RE.finditer(text):
            # Find the fn that follows this ignore within a few lines.
            window = text[m.end():m.end() + 200]
            fn = re.search(r"fn (\w+)\(", window)
            if fn and fn.group(1) in name.split("::")[-1:]:
                reason = m.group(1) or ""
                return reason.startswith("documented bug")
    return True  # unknown -> treat as witness (fail closed)


def self_test() -> int:
    total, witnesses = count_ignores()
    assert total >= witnesses >= 0, "witness subset invariant"
    # The pin file must parse as a non-negative integer.
    pin = int(PIN_FILE.read_text().strip())
    assert pin >= 0
    print(f"self-test ok: {total} ignores ({witnesses} witnesses), pin {pin}")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("--update-pin", action="store_true",
                    help="rewrite the pin to the current count (use only "
                         "after activating witnesses; the pin may only go "
                         "down between commits)")
    args = ap.parse_args()
    if args.self_test:
        return self_test()

    total, witnesses = count_ignores()
    pin = int(PIN_FILE.read_text().strip())

    if args.update_pin:
        if total > pin:
            print(f"REFUSING to raise the pin ({pin} -> {total}): ignores "
                  "may only be removed, not added")
            return 1
        PIN_FILE.write_text(f"{total}\n")
        print(f"pin updated: {pin} -> {total}")
        return 0

    ok = True
    if total > pin:
        print(f"FAIL: ignore count rose above the pin ({total} > {pin}). "
              "Every new #[ignore] must replace an activated witness or "
              "come with a pin update in the same commit, justified in "
              "its message.")
        ok = False
    else:
        print(f"count: {total} ignores ({witnesses} witnesses) <= pin {pin}")

    passing, failing = run_ignored()
    if passing is None:
        return 1
    dead = [t for t in passing if is_witness(t)]
    print(f"liveness: {len(failing)} witnesses still fail (correct), "
          f"{len(passing)} ignored tests pass")
    if dead:
        print("FAIL: passing bug-witnesses (activate or delete them):")
        for t in dead:
            print(f"  {t}")
        ok = False

    print("pbt-ignore-ratchet:", "PASS" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
