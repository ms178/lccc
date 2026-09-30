#!/usr/bin/env python3
"""Exhaustive tests for the relocation cross-check's decision logic.

`run_linker_tests.py` decides PASS/FAIL on `reloc_*_out_of_range_diagnosed_*`
from two functions: `reloc_oracle_verdict` (how one oracle linker's answer
counts) and `reloc_oracle_agreement` (how the set of answers folds into a
verdict).  Both decide whether lccc conforms to the ecosystem on a
psABI-mandated diagnostic, and both are therefore worth testing *directly*
rather than only through the fixture:

  * The **fixture route needs three linkers installed and a real `.o`** whose
    relocation is out of range.  On a host with only bfd, the interesting
    paths (two oracles going `inapplicable`, one crashing) never execute at
    all -- so a suite that only tested them end-to-end would report coverage
    it did not have.

  * These functions are precisely where the gate can **weaken silently**:
    `inapplicable` removes an oracle from the agreement set, so a bug here
    converts "three linkers agree" into "one linker agrees" without any test
    going red.  That failure mode is invisible from the outside by
    construction -- the whole point of `inapplicable` is that the excluded
    oracle said nothing -- so it has to be pinned from the inside.

Every case below is a known-answer case: the expected verdict is derived from
the psABI contract, not from the implementation.

Usage:
    python3 tests/linker/test_reloc_oracle_verdict.py
"""
from __future__ import annotations

import importlib.util
import os
import sys


def _load():
    """Import `run_linker_tests.py` by path.

    It is a script, not a package module, and its name does not end in a
    package importable from the repo root, so a plain `import` would fail
    depending on the caller's cwd.  Loading by path makes this test runnable
    from anywhere, which a gate script must be.
    """
    here = os.path.dirname(os.path.abspath(__file__))
    path = os.path.join(here, "run_linker_tests.py")
    spec = importlib.util.spec_from_file_location("_lccc_run_linker_tests", path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


RLT = _load()
verdict = RLT.reloc_oracle_verdict
agreement = RLT.reloc_oracle_agreement

# The three-oracle set the suite configures when mold and lld are installed.
THREE = [("bfd", ["gcc", "-fuse-ld=bfd"]),
         ("mold", ["gcc", "-fuse-ld=mold"]),
         ("wild", ["gcc", "-B/shim"])]
ONE = [("bfd", ["gcc", "-fuse-ld=bfd"])]
# The real host set: lld is the oracle that implements linker scripts.
THREE_LLD = [("bfd", ["gcc", "-fuse-ld=bfd"]),
             ("lld", ["gcc", "-fuse-ld=lld"]),
             ("mold", ["gcc", "-fuse-ld=mold"])]
TWO = [("bfd", ["gcc", "-fuse-ld=bfd"]), ("mold", ["gcc", "-fuse-ld=mold"])]

BFD_TRUNC = b"p.o: relocation R_X86_64_PC32 against symbol `farpc' can not be" \
            b" used; recompile with -fPIC: relocation truncated to fit"
MOLD_RANGE = b"mold: error: p.o: R_X86_64_PC32 out of range"
LLD_RANGE = b"ld.lld: error: relocation R_X86_64_PC32 out of range"
MOLD_SCRIPT = b"mold: error: t.ld: unknown linker script token"


def _notes(**kw):
    """Build the `notes` mapping the way the caller does: name -> (verdict, rc, stderr)."""
    return kw


# ── reloc_oracle_verdict ────────────────────────────────────────────────────
# (label, kind, stderr, rc, expected, why)
MOLD_SCRIPT = (b"mold: fatal: t.ld:1: ENTRY(probe)\n"
              b"                     ^ unknown linker script token\n"
              b"collect2: error: ld returned 1 exit status 1")

VERDICT_CASES = [
    # ── exit status dominates everything ──
    ("exec", "PC32", BFD_TRUNC, 0, "accepted",
     "rc 0 means it linked: lccc refuses what the ecosystem accepts"),
    ("exec", "PC32", b"", 0, "accepted",
     "rc 0 with empty stderr is still a successful link"),
    ("exec", "PC32", BFD_TRUNC, 139, "errored",
     "139 = SIGSEGV: a crash is a MISSING result, never 'no opinion'"),
    ("exec", "PC32", BFD_TRUNC, 134, "errored",
     "134 = SIGABRT"),
    ("exec", "PC32", b"", 137, "errored",
     "137 = SIGKILL, i.e. the OOM killer: must not look like inapplicable"),
    ("exec", "PC32", b"", 2, "errored",
     "rc 2 is not a linker diagnostic exit; treat as an error"),
    ("exec", "PC32", b"", 127, "errored",
     "127 = command not found: a broken runner, not a neutral oracle"),
    # `errored` must beat the `shared` shortcut, which accepts ANY refusal.
    ("shared", "32", b"", 139, "errored",
     "the shared shortcut says any refusal counts, but a segfault is not a refusal"),
    ("shared", "32", b"anything at all", 1, "refused",
     "shared path: GNU ld words its R_X86_64_32 refusal its own way"),
    ("shared", "32", b"", 1, "refused",
     "shared path: even an unexplained refusal counts"),
    # ── `incapable`: the fixture was rejected before any relocation ──
    # These exercise the PRODUCER, not the agreement function.  The
    # `incapable` class was, until now, reachable only by hand-writing it
    # into a notes dict -- so every `incapable` case below the agreement
    # function passed while the real pipeline could never emit the class.
    # The recorded mold 2.37.1 stderr, verbatim.
    ("script", "PC32", MOLD_SCRIPT, 1, "incapable",
     "mold cannot parse the minimal -T script: a missing FEATURE, so the "
     "quorum is taken over the oracles that could have answered"),
    # The same stderr on a path with no script must NOT be excused: the
    # script mention is a red herring there, and silently narrowing the
    # quorum off a path that never had a script would be exactly the
    # false-green the split exists to prevent.
    ("exec", "PC32", MOLD_SCRIPT, 1, "inapplicable",
     "no script on this path, so 'linker script' in the text excuses nothing"),
    ("shared", "32", MOLD_SCRIPT, 1, "refused",
     "the shared shortcut still wins: any refusal counts"),
    # Naming the relocation beats the script excuse -- if the oracle got as
    # far as discussing R_X86_64_PC32 it clearly reached it, and the
    # fixture was not what stopped it.
    ("script", "PC32", BFD_TRUNC + b" (while reading t.ld)", 1, "refused",
     "a refusal that NAMES the relocation is an opinion even if a script "
     "is also mentioned: the oracle demonstrably reached the relocation"),
    # ── the wording is not part of the contract ──
    ("exec", "PC32", BFD_TRUNC, 1, "refused",
     "bfd's spelling: 'relocation truncated to fit'"),
    ("exec", "PC32", MOLD_RANGE, 1, "refused",
     "mold's spelling: 'out of range' -- the psABI mandates neither"),
    ("exec", "PC32", LLD_RANGE, 1, "refused",
     "lld's spelling: 'out of range'"),
    ("script", "PC32", MOLD_RANGE, 1, "refused",
     "same on the linker-script path"),
    # ── inapplicable: never reached the relocation ──
    ("exec", "PC32", MOLD_SCRIPT, 1, "inapplicable",
     "mold cannot parse the -T script, so it never reached the relocation"),
    ("exec", "PC32", b"", 1, "inapplicable",
     "no mention of the relocation type at all"),
    ("exec", "PC32", b"mold: error: undefined symbol: probe", 1, "inapplicable",
     "a DIFFERENT error means the relocation was never evaluated"),
    # Kind matters: naming another relocation type is not naming this one.
    ("exec", "32", BFD_TRUNC, 1, "inapplicable",
     "the diagnostic names PC32 but the case is R_X86_64_32"),
    # ── silent: refused, named the type, but no range failure ──
    ("exec", "PC32", b"p.o: relocation R_X86_64_PC32: bad value", 1, "silent",
     "named the type but no range failure: an unrecognised refusal is NOT conformity"),
    ("exec", "PC32", b"p.o: R_X86_64_PC32 against 'farpc': unsupported", 1, "silent",
     "same, mold-flavoured wording"),
]


# ── reloc_oracle_agreement ──────────────────────────────────────────────────
def _agree(notes):
    """Rebuild the `agree` list exactly as the caller does.

    `incapable` is `continue`d by the caller before an oracle ever runs, so it
    must not be turned into a `(name, False)` disagreement here either.
    """
    return [(n, v == "refused") for n, (v, _, _) in notes.items()
            if v not in ("inapplicable", "incapable")]


def _case(name, notes, oracles, want_status, want_substring=""):
    status, detail = agreement(_agree(notes), notes, oracles)
    ok = status == want_status
    if want_substring and want_substring not in detail:
        ok = False
    return name, ok, "status=%r detail=%r (want %r%s)" % (
        status, detail, want_status,
        " containing %r" % want_substring if want_substring else "")


def _n(v, rc=1, err=b"x"):
    return (v, rc, err.decode())


AGREEMENT_CASES = [

    # ── the happy path ──
    _case("all three refuse",
          _notes(bfd=_n("refused"), mold=_n("refused"), wild=_n("refused")),
          THREE, "PASS"),
    _case("single-oracle host still passes on its one opinion",
          _notes(bfd=_n("refused")), ONE, "PASS"),
    # ── real disagreements fail ──
    _case("one oracle accepts what lccc refuses",
          _notes(bfd=_n("refused"), mold=_n("accepted", 0), wild=_n("refused")),
          THREE, "FAIL", "disagree"),
    _case("an unrecognised refusal is a disagreement",
          _notes(bfd=_n("refused"), mold=_n("silent"), wild=_n("refused")),
          THREE, "FAIL", "disagree"),
    _case("a crashed oracle is a disagreement, not an absence",
          _notes(bfd=_n("refused"), mold=_n("errored", 139), wild=_n("refused")),
          THREE, "FAIL", "disagree"),
    # ── the floor: the audit's finding ──
    _case("two inapplicable out of three is NOT a cross-check",
          _notes(bfd=_n("refused"), mold=_n("inapplicable"),
                 wild=_n("inapplicable")),
          THREE, "FAIL", "need 2"),
    _case("all three inapplicable fails",
          _notes(bfd=_n("inapplicable"), mold=_n("inapplicable"),
                 wild=_n("inapplicable")),
          THREE, "FAIL", "need 2"),
    # A crash does NOT rescue the floor: `errored` stays in the set, but it is
    # a disagreement -- it must never be counted as an applicable opinion.
    _case("crash + inapplicable still fails the floor",
          _notes(bfd=_n("refused"), mold=_n("errored", 139),
                 wild=_n("inapplicable")),
          THREE, "FAIL"),
    # ── the reference must have an opinion ──
    _case("reference inapplicable fails even with a quorum",
          _notes(bfd=_n("inapplicable"), mold=_n("refused"), wild=_n("refused")),
          THREE, "FAIL", "reference oracle"),
    # ── degradation is visible, not silent ──
    _case("one inapplicable out of three passes but says so",
          _notes(bfd=_n("refused"), mold=_n("inapplicable"), wild=_n("refused")),
          THREE, "PASS", "agreed by 2 of 3"),
    # ── an empty oracle set must FAIL, never PASS ────────────────────────
    # Before the fix this returned PASS with zero evidence: floor = min(2, 0)
    # = 0 so `len(applicable) < floor` was `0 < 0` = False, `reference` was
    # None so the reference check was skipped, and `all([])` is True. Every
    # guard was satisfied by the absence of any oracle at all. A gate that
    # exists to refuse verdicts resting on too little evidence must not
    # certify one resting on none.
    _case("no oracles configured fails instead of passing vacuously",
          _notes(), [], "FAIL", "no oracles"),
    # Same trap, reached with notes present but no oracle to vouch for them:
    # the notes must not be mistaken for evidence.
    _case("notes without any configured oracle still fails",
          _notes(bfd=_n("refused"), mold=_n("refused")), [], "FAIL", "no oracles"),
    # A third route to the same shape: notes that CLAIM agreement, with no
    # oracle configured to back the claim. The notes are not evidence, and a
    # reader scanning a passing report should not be able to mistake them for
    # a quorum. Distinct from the case above, which seeds refused notes.
    _case("an empty oracle set fails even if notes claim agreement",
          _notes(bfd=_n("refused")), [], "FAIL", "no oracles"),
]
AGREEMENT_CASES += [
    # ── `incapable`: a missing FEATURE (probed structurally), not a missing
    #    opinion.  mold advertises -T but cannot parse even a minimal
    #    SECTIONS, so on a host with mold installed the script-path
    #    cross-checks used to drop to a single opinion and fail.  These pin
    #    the fix: exclude the feature gap, keep the quorum over the oracles
    #    that could answer, and never let the exclusion hide a disagreement.
    _case("an incapable oracle is excluded; quorum is over the capable ones",
          _notes(bfd=_n("refused"), lld=_n("refused"),
                 mold=_n("incapable", 1, b"cannot parse a -T linker script")),
          THREE_LLD, "PASS", "not counted"),
    _case("incapable does not rescue a disagreement",
          _notes(bfd=_n("refused"), lld=_n("accepted", 0),
                 mold=_n("incapable", 1, b"cannot parse a -T linker script")),
          THREE_LLD, "FAIL", "disagree"),
    _case("incapable does not rescue a silent refusal",
          _notes(bfd=_n("refused"), lld=_n("silent"),
                 mold=_n("incapable", 1, b"cannot parse a -T linker script")),
          THREE_LLD, "FAIL", "disagree"),
    _case("every oracle incapable is not a vacuous PASS",
          _notes(bfd=_n("incapable", 1, b"cannot parse a -T linker script"),
                 lld=_n("incapable", 1, b"cannot parse a -T linker script"),
                 mold=_n("incapable", 1, b"cannot parse a -T linker script")),
          THREE_LLD, "FAIL"),
    _case("incapable narrows the quorum honestly, and says so",
          _notes(bfd=_n("refused"), mold=_n("incapable", 1, b"no -T support")),
          TWO, "PASS", "agreed by 1 of 2"),
    # An `inapplicable` oracle could still have answered, so it does NOT
    # shrink the quorum -- only a structural `incapable` does.  Here lld could
    # have had an opinion and merely did not, so the floor stays 2 and bfd's
    # lone refusal is correctly not enough.
    _case("inapplicable still counts against the quorum; only incapable frees a seat",
          _notes(bfd=_n("refused"), lld=_n("inapplicable"),
                 mold=_n("incapable", 1, b"no -T support")),
          THREE_LLD, "FAIL", "need 2"),
    _case("the reference being incapable still fails",
          _notes(bfd=_n("incapable", 1, b"no -T support"),
                 lld=_n("refused"), mold=_n("refused")),
          THREE_LLD, "FAIL"),
]



REQUIRED_CASES = [
    # (label, oracles, required, want_missing)
    ("a fully stocked host satisfies bfd,lld",
     [("bfd", []), ("lld", []), ("mold", [])], ["bfd", "lld"], []),
    ("a host without lld is caught -- the silent-narrowing case",
     [("bfd", []), ("mold", [])], ["bfd", "lld"], ["lld"]),
    ("bfd alone cannot satisfy a two-oracle quorum",
     [("bfd", [])], ["bfd", "lld"], ["lld"]),
    ("mold present but lld absent is still missing",
     [("bfd", []), ("mold", []), ("wild", [])], ["lld"], ["lld"]),
    ("an empty requirement is always satisfied",
     [("bfd", [])], [], []),
    ("every missing name is reported, not just the first",
     [("bfd", [])], ["bfd", "lld", "mold"], ["lld", "mold"]),
]


def _required_cases(RLT):
    out = []
    for label, oracles, required, want in REQUIRED_CASES:
        got = RLT.missing_required_oracles(oracles, required)
        ok = got == want
        detail = "" if ok else "want %r, got %r" % (want, got)
        out.append((label, ok, detail))
    return out


def main() -> int:
    failures = 0
    print("== reloc_oracle_verdict ==")
    for label, kind, err, rc, want, why in VERDICT_CASES:
        got = verdict(err, rc, kind, label)
        ok = got == want
        failures += 0 if ok else 1
        print("  %-4s %-8s kind=%-4s rc=%-4d -> %-13s %s" % (
            "ok" if ok else "FAIL", label, kind, rc, got, why))
    print("== reloc_oracle_agreement ==")
    for name, ok, detail in AGREEMENT_CASES:
        failures += 0 if ok else 1
        print("  %-4s %s" % ("ok" if ok else "FAIL", name))
        if not ok:
            print("        %s" % detail)
    print("== missing_required_oracles ==")
    req = _required_cases(RLT)
    for name, ok, detail in req:
        failures += 0 if ok else 1
        print("  %-4s %s" % ("ok" if ok else "FAIL", name))
        if not ok:
            print("        %s" % detail)
    print()
    total = len(VERDICT_CASES) + len(AGREEMENT_CASES) + len(req)
    if failures:
        print("FAILED: %d of %d cases" % (failures, total))
        return 1
    print("PASS: all %d cases (%d verdict, %d agreement, %d required)" % (
        total, len(VERDICT_CASES), len(AGREEMENT_CASES), len(req)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
