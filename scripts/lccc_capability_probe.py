#!/usr/bin/env python3
"""Establish LCCC's driver capabilities from evidence, for the Clang-C corpus.

Why this exists
---------------
``tests/corpus/run_clang_c_corpus.py`` is deliberately conservative: an
invocation whose flags the compiler has not been *shown* to accept is reported
UNSUPPORTED rather than executed.  For ``--compiler-family clang`` the accepted
flag set is a static table (``tools/corpus/directives.KNOWN_FLAGS``); for any
other family the harness demands that capabilities be established explicitly
via ``--capability``.  Without that evidence the corpus grid is empty: measured
on 2026-10-03 against a fastbuild ``lccc``, only 102 of 5 130 planned
invocations executed and 5 028 were filtered, so the corpus exercised almost
nothing.

This tool produces the missing evidence.  It does NOT guess from flag spelling
and it does NOT inherit Clang's table.  Every capability is established by
running the compiler under test and recording the command and its exit status,
so the resulting manifest can be re-verified and cannot silently drift from the
binary it describes.

Capability contracts established here
-------------------------------------
``flag:X``
    *Driver acceptance*: ``lccc -fsyntax-only -x c <empty> X`` exits 0.  LCCC
    refuses an option it has no implementation for (matching GCC 14/16, Clang
    23.1, ICC 2021.10 and ICX, measured; see ``src/driver/cli.rs``), so the
    exit status is the predicate wanted here without any environment help.

    ``LCCC_STRICT_OPTIONS=1`` is exported anyway, and it is what makes the
    probe *sound* rather than merely convenient: LCCC's deliberately tolerated
    class -- ``-Wno-<unknown>``, ``--param <name>=<v>``, ``-g<selector>`` --
    is accepted with a diagnostic by default because GCC accepts the first
    outright, and under strict options it is fatal.  A spelling in that class
    must therefore NOT be reported as established, and only the strict run can
    tell it apart from an implemented option.

    What this does and does not prove: it proves the driver has an
    implementation path for the flag.  It does not prove the flag's semantics
    match Clang's in every corner -- the corpus run is what tests that, and a
    FAIL from this manifest is a genuine defect to triage rather than a harness
    artifact.

``clang-driver-target``
    The corpus needs the *default* target to decide whether a
    non-``-target=``-qualified RUN is answering about x86-64.  Established by
    ``-dumpmachine`` returning an x86-64 triple.

``clang-default-c-dialect``
    A RUN with no ``-std=`` is written against Clang's default dialect.  LCCC
    defaults to ``__STDC_VERSION__ 201710L`` (C17), the same as Clang 23 on
    x86-64 Linux, so an unqualified RUN means the same thing to both compilers.
    Established by preprocessing ``-dM`` and reading the reported version --
    and that is *all* it proves: a correct macro next to a parser that never
    implemented the dialect would still pass this probe, which is why the
    manifest records it as a macro-value claim and the corpus run, not this
    tool, is where dialect semantics are exercised.

Anything else (``external-inputs``, EDG-only associations, ...) is NOT
probeable by compilation and is therefore never claimed.

Negative controls
-----------------
The probe's whole validity rests on an unimplemented option being refused, so
it does not test that premise once with one spelling: it refuses to emit a
manifest unless *every* control spelling below is rejected by the compiler
under test (``--lccc-probe-...-xyzzy``, ``-W...``, ``-Werror=...``,
``-f...``), **and** unless the tolerated class is refused under the strict
options this probe exports (``-Wno-...``, which GNU accepts outright).  A
driver that accepts any of them cannot separate "implemented" from "ignored",
and every capability it reported would be worthless.  See
``NEGATIVE_CONTROLS`` for the per-compiler measurements.

Coverage
--------
``coverage.runnable_invocations`` counts planned invocations whose *every*
demanded capability is established -- the corpus runner's own per-invocation
rule -- rather than summing capability occurrences.  ``blocked_by`` records how
many invocations each unestablished capability blocks, so a low ratio is
diagnosable from the manifest alone.  Capabilities this tool deliberately
refuses to claim (``not_probeable``) count as blocking, so the ratio is what
can be run, never what can be argued.

Reproducibility
---------------
The manifest pins the compiler path, its SHA-256, the LCCC version banner, the
probe protocol version, every probe's outcome (supported flag, exit status and
reason) and an ``outcomes_sha256`` over those outcomes, so a rerun after a
compiler change either reproduces the manifest or can be diffed to the exact
capabilities that moved.  ``--emit-runner-args`` writes the ``--capability X``
pairs that ``run_clang_c_corpus.py`` accepts (one token per line, ready for
``xargs -d '\n'``); ``--emit-args`` prints the bare capability names for tools
that take a list.

Usage
-----
    # Write an evidence manifest for one compiler.
    scripts/lccc_capability_probe.py --cc target/fastbuild/lccc \\
        --manifest results/caps.json

    # Feed the corpus runner directly (one token per line).
    scripts/lccc_capability_probe.py --cc target/fastbuild/lccc --emit-runner-args \\
        | xargs -d '\\n' python3 tests/corpus/run_clang_c_corpus.py \\
              --cc target/fastbuild/lccc --compiler-family lccc --mode outcome-only
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile

REPO = Path(__file__).resolve().parents[1]
DEFAULT_INDEX = REPO / "tests" / "corpus" / "clang-c" / "corpus-index.json"

# Bump when the meaning of a probe changes, so a stale manifest is detectable
# rather than silently reused.  The corpus runner's expectations are pinned the
# same way (tests/corpus/clang-c/corpus-index.json has its own schema id).
PROTOCOL = 3

# Capabilities that describe the driver rather than a single flag.  Each is
# established by a dedicated probe below; none is assumed from the family name.
DRIVER_CAPABILITIES = ("clang-driver-target", "clang-default-c-dialect")

# Capabilities the corpus index can demand that are NOT decidable by invoking
# the compiler, with the reason.  Listed so that "we did not claim it" is a
# recorded decision and not an oversight.
NOT_PROBEABLE = {
    "external-inputs": "requires resolving %S/%t side inputs outside the corpus root",
}

# Control spellings that must be REFUSED by the compiler under test for the
# evidence to mean anything.  Measured 2026-10-04 on the pinned oracles
# (GCC 16.2, Clang 23.1, ICC 2021.10, ICX) and on GCC 14.2 in this tree:
#
#   spelling            gcc16.2 clang23 icc21 icx   lccc (default / strict)
#   --<unknown>         FAIL    FAIL    OK    FAIL  accepted / FAIL
#   -f<unknown>         FAIL    FAIL    OK    FAIL  accepted / FAIL
#   -W<unknown>         FAIL    OK      OK    OK    accepted / FAIL
#   -Werror=<unknown>   FAIL    OK      OK    OK    accepted / FAIL
#   -Wno-<unknown>      OK      OK      OK    OK    accepted / accepted
#
# The middle two rows are why the strict export is not a formality: LCCC
# diagnoses an unknown warning name rather than failing the build (it has six
# implemented warning names against GNU's 409), so only the strict run turns
# that tolerance into the refusal the control demands.
#
# One control spelling pins one code path, and the `-W` family is where a
# driver most plausibly accepts an option it never implemented: the LCCC driver
# used to discard its warning-config parser's rejection, so a `-Wbogus`
# capability would have been blessed by a `--`-only control.  Refusal of the
# `-W` spellings is a statement about the compiler under test's own contract
# (LCCC follows GCC, which is the only one of the four that refuses them), not
# about C compilers in general -- which is exactly why it must be *measured*
# per compiler rather than assumed.
NEGATIVE_CONTROLS = (
    ("--lccc-probe-nonexistent-option-xyzzy", "unknown long option"),
    ("-Wlccc-probe-nonexistent-warning", "unknown warning name"),
    ("-Werror=lccc-probe-nonexistent-warning", "unknown warning name under -Werror="),
    ("-flccc-probe-nonexistent-flag", "unknown -f option"),
)

# Control spellings that must be ACCEPTED even with strict options exported,
# because the request is satisfied by construction rather than tolerated:
# `-Wno-<unknown>` asks for a warning to be off and no such warning exists
# (all four reference compilers accept it, measured).  A compiler that fails
# this one is not honouring the off-request class, which is the other half of
# the contract the controls pin -- the probe would otherwise be free to report
# every `-Wno-...` capability as unsupported for the wrong reason.
ACCEPTED_CONTROLS = (("-Wno-lccc-probe-nonexistent-warning", "off-request class"),)


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for block in iter(lambda: fh.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def _capability_counts(doc: dict) -> dict[str, int]:
    """Every capability the selected corpus can demand, with its frequency."""
    counts: dict[str, int] = {}
    for rec in doc["files"]:
        for inv in rec["invocations"]:
            for cap in inv["capabilities"]:
                counts[cap] = counts.get(cap, 0) + 1
    return counts


def coverage_of(doc: dict, supported: set[str]) -> dict:
    """How much of the selected corpus the established capabilities unlock.

    The corpus runner decides per INVOCATION, not per capability: an invocation
    is executed only when every capability it names is established (or is on
    the static known-flag table).  Summing capability occurrences would report
    the wrong number twice over -- it credits a capability that shares every
    invocation with an unestablished one, and it inflates by however many
    invocations happen to list a flag more than once.  So the metric here is
    distinct planned invocations that are fully unlocked, and each capability's
    blocking weight is recorded so the ratio can be explained without rerunning
    anything.
    """
    planned = 0
    runnable = 0
    blocked_by: dict[str, int] = {}
    for rec in doc["files"]:
        for inv in rec["invocations"]:
            planned += 1
            missing = sorted({c for c in inv["capabilities"] if c not in supported})
            if missing:
                for cap in missing:
                    blocked_by[cap] = blocked_by.get(cap, 0) + 1
            else:
                runnable += 1
    return dict(
        planned_invocations=planned,
        runnable_invocations=runnable,
        ratio=(runnable / planned) if planned else 0.0,
        blocked_by=dict(sorted(blocked_by.items())),
    )


def outcomes_sha256(results: dict[str, dict]) -> str:
    """One hash over every probe outcome, so a manifest can be compared at a
    glance and a diff can be localised by reading the manifest itself."""
    payload = [
        [cap, bool(r.get("supported")), r.get("rc")]
        for cap, r in sorted(results.items())
    ]
    blob = json.dumps(payload, separators=(",", ":"), sort_keys=True)
    return hashlib.sha256(blob.encode("utf-8")).hexdigest()


def run(argv: list[str], env: dict[str, str], stdin: bytes = b"", timeout: float = 30.0):
    """Run one probe. Returns (returncode, stdout, stderr, timed_out)."""
    try:
        proc = subprocess.run(
            argv,
            input=stdin,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=env,
            timeout=timeout,
        )
    except subprocess.TimeoutExpired:
        return None, "", "", True
    return proc.returncode, proc.stdout.decode("utf-8", "replace"), proc.stderr.decode(
        "utf-8", "replace"
    ), False


def probe_flag(cc: list[str], flag: str, env: dict[str, str], timeout: float) -> dict:
    """Is `flag` implemented by this driver?

    Runs against an empty translation unit so the probe cannot fail for a
    reason belonging to any real test, and uses stdin (`-`) so no probe ever
    writes a file next to the corpus.
    """
    argv = cc + ["-fsyntax-only", "-x", "c", "-", flag]
    rc, _out, err, timed_out = run(argv, env, stdin=b"int _lccc_probe;\n", timeout=timeout)
    if timed_out:
        return dict(supported=False, why="probe timed out", rc=None)
    if rc == 0:
        return dict(supported=True, why="driver accepted the flag", rc=0)
    # Distinguish "flag unimplemented" from "flag rejected for another reason".
    first = next((ln for ln in err.splitlines() if ln.strip()), "")
    unimplemented = "unrecognized command-line option" in first
    return dict(
        supported=False,
        why="unimplemented option (strict probe)" if unimplemented else "rejected: " + first[:200],
        rc=rc,
    )


def probe_driver_capabilities(
    cc: list[str], env: dict[str, str], timeout: float
) -> dict[str, dict]:
    out: dict[str, dict] = {}

    rc, machine, _err, _to = run(cc + ["-dumpmachine"], env, timeout=timeout)
    machine = machine.strip()
    # x86_64 / amd64 are the two spellings Clang and GCC use for the same
    # default the corpus assumes when a RUN carries no -target=.
    ok = rc == 0 and machine.split("-", 1)[0] in {"x86_64", "amd64"}
    out["clang-driver-target"] = dict(
        supported=ok,
        why=f"-dumpmachine -> {machine!r}" if machine else "no -dumpmachine output",
        rc=rc,
        triple=machine,
    )

    rc, macros, _err, _to = run(cc + ["-dM", "-E", "-x", "c", "-"], env, stdin=b"", timeout=timeout)
    version = ""
    for line in macros.splitlines():
        if line.startswith("#define __STDC_VERSION__"):
            version = line.split()[-1].strip("L")
    # Clang 23 on x86-64 Linux defaults to C17, i.e. 201710L.
    ok = rc == 0 and version == "201710"
    out["clang-default-c-dialect"] = dict(
        supported=ok,
        why=f"__STDC_VERSION__ {version or '<absent>'}" + ("" if ok else " (expected 201710)"),
        rc=rc,
        stdc_version=version,
    )
    return out


# --------------------------------------------------------------------------
# Selftest
# --------------------------------------------------------------------------
#
# Drives the whole probe against a *synthetic* compiler that reproduces the
# exact contract the tool depends on, so the contract is pinned without needing
# a real LCCC (or any C toolchain) in CI:
#
#   * `-dumpmachine`            -> an x86-64 triple
#   * `-dM -E`                  -> `__STDC_VERSION__ 201710L`
#   * a known flag              -> exit 0 silently
#   * an unknown `--`/`-W`/`-f` flag
#                               -> exit 1 with "unrecognized command-line
#                                  option" (measured GCC 14/16, Clang 23.1,
#                                  ICC 2021.10, ICX)
#   * `-Wno-<unknown>`          -> exit 0 with a warning, unless
#                                  LCCC_STRICT_OPTIONS is exported, in which
#                                  case exit 1 (the tolerated class)
#
# The last two are the properties the whole tool rests on: an unimplemented
# option must not read as support, and the tolerated class must not be blessed
# either.  The selftest asserts both, including the mutations that break them.
#
# Environment switches used by the selftest (never set in a real run):
#   FAKE_TRIPLE / FAKE_STDC     - the reported target / dialect
#   FAKE_ACCEPTS_UNKNOWN_W=1    - accept `-W`-spelled unknowns (mutates the
#                                 premise; the `-W` negative control must catch it)
#   FAKE_ACCEPTS_STRICT=1       - ignore LCCC_STRICT_OPTIONS entirely
#   FAKE_TOLERATES_UNDER_STRICT=1
#                               - keep accepting `-Wno-*` under strict options

FAKE_COMPILER = r"""#!/usr/bin/env python3
import os, sys
args = sys.argv[1:]
KNOWN = {"-mfake-known"}
if "--version" in args:
    print("fakecc 1.0"); raise SystemExit(0)
if "-dumpmachine" in args:
    print(os.environ.get("FAKE_TRIPLE", "x86_64-linux-gnu")); raise SystemExit(0)
if "-dM" in args:
    print("#define __STDC_VERSION__ " + os.environ.get("FAKE_STDC", "201710L"))
    raise SystemExit(0)
PASS_THROUGH = {"-fsyntax-only", "-x", "c", "-"}
strict = bool(os.environ.get("LCCC_STRICT_OPTIONS"))
if os.environ.get("FAKE_ACCEPTS_STRICT") == "1":
    strict = False
unknown = [a for a in args if a.startswith("-") and a not in KNOWN
           and a not in PASS_THROUGH]
if unknown:
    flag = unknown[0]
    off_request = flag.startswith("-Wno-")
    if off_request:
        # Satisfied by construction: nothing by that name exists here, so the
        # request is honoured even under strict options.
        if os.environ.get("FAKE_TOLERATES_UNDER_STRICT") == "1":
            sys.stderr.write("ccc: error: unrecognized command-line option '%s'\n" % flag)
            raise SystemExit(1)
        raise SystemExit(0)
    if os.environ.get("FAKE_ACCEPTS_UNKNOWN_W") == "1":
        raise SystemExit(0)
    if not strict:
        sys.stderr.write("warning: unrecognized command-line option '%s'\n" % flag)
        raise SystemExit(0)
    sys.stderr.write("ccc: error: unrecognized command-line option '%s'\n" % flag)
    raise SystemExit(1)
raise SystemExit(0)
"""


def _selftest() -> int:
    import tempfile
    import unittest

    class ProbeContract(unittest.TestCase):
        def run_probe(self, *extra: str, env: dict[str, str] | None = None,
                      index_invocations=None):
            """Invoke this script as a subprocess against a synthetic compiler.

            ``index_invocations`` is a list of capability lists -- one entry per
            planned invocation, exactly the shape the corpus index uses -- so a
            test can decide which capabilities share an invocation (the
            per-invocation coverage rule) rather than assuming one each.

            Returns (returncode, stdout, stderr, manifest-or-None). The manifest
            is parsed INSIDE the temporary directory: returning its path would
            hand back a file that the context manager has already deleted.
            """
            with tempfile.TemporaryDirectory() as td:
                fake = Path(td) / "fakecc"
                fake.write_text(FAKE_COMPILER)
                fake.chmod(0o755)
                index = Path(td) / "index.json"
                invocations = index_invocations or [
                    ["flag:-mfake-known"],
                    ["flag:-mfake-unknown"],
                    ["clang-driver-target"],
                    ["clang-default-c-dialect"],
                ]
                index.write_text(
                    json.dumps(
                        {
                            "files": [
                                {
                                    "invocations": [
                                        {"capabilities": caps} for caps in invocations
                                    ]
                                }
                            ]
                        }
                    )
                )
                man = Path(td) / "manifest.json"
                proc = subprocess.run(
                    [
                        sys.executable,
                        str(Path(__file__).resolve()),
                        "--cc",
                        str(fake),
                        "--index",
                        str(index),
                        "--manifest",
                        str(man),
                        *extra,
                    ],
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    env={**os.environ, **(env or {})},
                    timeout=120,
                )
                doc = json.loads(man.read_text()) if man.is_file() else None
                return proc.returncode, proc.stdout.decode(), proc.stderr.decode(), doc

        def test_known_flag_established_unknown_refused(self):
            rc, out, err, doc = self.run_probe("--emit-args")
            self.assertEqual(rc, 0, err)
            established = out.split()
            self.assertIn("flag:-mfake-known", established)
            self.assertNotIn(
                "flag:-mfake-unknown",
                established,
                "an unknown flag must never be reported as established",
            )
            self.assertEqual(doc["unsupported"].keys(), {"flag:-mfake-unknown"})
            # And the refusal must carry a reason, not an empty placeholder.
            self.assertIn("unimplemented", doc["unsupported"]["flag:-mfake-unknown"]["why"])

        def test_every_control_spelling_is_required_to_fail(self):
            """The premise is per-spelling, and the run must say so.

            A driver that refuses the `--` spelling but accepts a `-W`-spelled
            unknown option still cannot distinguish implemented from ignored --
            that is the exact shape of the defect this control set exists for.
            """
            _rc, _out, _err, doc = self.run_probe()
            by_flag = {c["flag"]: c for c in doc["controls"]}
            self.assertEqual(
                set(by_flag),
                {flag for flag, _ in NEGATIVE_CONTROLS}
                | {flag for flag, _ in ACCEPTED_CONTROLS},
                "every registered control must be probed and recorded",
            )
            for flag, record in by_flag.items():
                if "why_it_must_fail" in record:
                    self.assertFalse(record["supported"], flag)
                    self.assertTrue(record["why_it_must_fail"], flag)
                else:
                    self.assertTrue(record["supported"], flag)

            # Mutation: a driver that accepts `-W`-spelled unknowns silently
            # must stop the probe, not be blessed by it.
            rc, _out, err, _doc = self.run_probe(env={"FAKE_ACCEPTS_UNKNOWN_W": "1"})
            self.assertEqual(rc, 2, "an accepted -W control must abort the probe")
            self.assertIn("control(s)", err)

        def test_off_request_class_must_be_honoured(self):
            """`-Wno-<unknown>` asks for a warning to be off and no such warning
            exists, so the request is satisfied -- it must be accepted even with
            strict options exported.  A compiler that refuses it (or that
            accepts it only because it ignores strict mode) is not honouring the
            class, and the probe must abort rather than certify flags against a
            contract it does not share."""
            rc, _out, err, doc = self.run_probe()
            self.assertEqual(rc, 0, err)
            by_flag = {c["flag"]: c for c in doc["controls"]}
            self.assertIn("-Wno-lccc-probe-nonexistent-warning", by_flag)
            self.assertTrue(by_flag["-Wno-lccc-probe-nonexistent-warning"]["supported"])
            self.assertEqual(
                by_flag["-Wno-lccc-probe-nonexistent-warning"]["why_it_must_pass"],
                "off-request class",
            )

            # Mutation: a compiler that refuses the satisfied off-request must
            # abort the probe.
            rc, _out, err, _doc = self.run_probe(
                env={"FAKE_TOLERATES_UNDER_STRICT": "1"}
            )
            self.assertEqual(rc, 2, "an off-request refusal must abort the probe")
            self.assertIn("-Wno-", err)

        def test_strict_env_is_actually_exported(self):
            """Without the export an unimplemented option is only a warning in
            the default policy, so every flag would probe 'true'."""
            rc, _out, err, _doc = self.run_probe(env={"FAKE_ACCEPTS_STRICT": "1"})
            self.assertEqual(
                rc, 2, "the negative control must abort when strict mode is not honoured"
            )
            self.assertIn("control(s)", err)

        def test_driver_capabilities_are_measured_not_assumed(self):
            rc, _out, err, doc = self.run_probe()
            self.assertEqual(rc, 0, err)
            self.assertIn("clang-driver-target", doc["supported"])
            self.assertIn("clang-default-c-dialect", doc["supported"])
            self.assertEqual(doc["demanded"]["flag:-mfake-known"], 1)

            # A non-x86-64 default target must NOT be claimed as the corpus's
            # x86-64 assumption.
            _rc, _o, _e, doc = self.run_probe(env={"FAKE_TRIPLE": "aarch64-linux-gnu"})
            self.assertNotIn("clang-driver-target", doc["supported"])

            # A different default dialect must NOT be claimed either.
            _rc, _o, _e, doc = self.run_probe(env={"FAKE_STDC": "202311L"})
            self.assertNotIn("clang-default-c-dialect", doc["supported"])

        def test_coverage_is_per_invocation_not_a_capability_sum(self):
            """The runner executes an invocation only when EVERY capability it
            names is established, so the manifest must count invocations that
            are fully unlocked -- not occurrences of established capabilities.

            The index below is the adversarial shape: `flag:-mfake-known` occurs
            in three invocations but only one of them is fully unlocked, so a
            weighted-sum metric would report 3/4 = 75% where the truth is 25%.
            """
            rc, _out, err, doc = self.run_probe(
                index_invocations=[
                    ["flag:-mfake-known"],
                    ["flag:-mfake-known", "flag:-mfake-unknown"],
                    ["flag:-mfake-known", "clang-driver-target"],
                    ["flag:-mfake-unknown"],
                ]
            )
            self.assertEqual(rc, 0, err)
            cov = doc["coverage"]
            self.assertEqual(cov["planned_invocations"], 4)
            self.assertEqual(
                cov["runnable_invocations"],
                2,
                "only the single-capability and the driver-capability invocation "
                "have every demanded capability established",
            )
            self.assertAlmostEqual(cov["ratio"], 0.5)
            # The blocker weight must be loud enough to explain the ratio.
            self.assertEqual(cov["blocked_by"]["flag:-mfake-unknown"], 2)
            self.assertEqual(doc["demanded"]["flag:-mfake-known"], 3)

        def test_runner_arg_shapes(self):
            """--emit-runner-args must produce '--capability <X>' PAIRS; a bare
            name list is rejected by argparse in the corpus runner."""
            rc, out, err, _doc = self.run_probe("--emit-runner-args")
            self.assertEqual(rc, 0, err)
            # Drop the empty tail produced by the final newline before pairing.
            lines = [ln for ln in out.split("\n") if ln != ""]
            self.assertEqual(len(lines) % 2, 0, f"pairs must be balanced: {lines!r}")
            pairs = list(zip(lines[::2], lines[1::2]))
            self.assertTrue(pairs, "at least one capability must be emitted")
            for head, value in pairs:
                self.assertEqual(head, "--capability")
                self.assertNotEqual(value, "")
            self.assertIn(("--capability", "flag:-mfake-known"), pairs)
            # ...and the bare-name mode used by list-taking tools is the other
            # documented shape, not an alias of the pair mode.
            rc, out, err, _doc = self.run_probe("--emit-args")
            self.assertEqual(rc, 0, err)
            self.assertIn("flag:-mfake-known", out.split())
            self.assertNotIn("--capability", out.split())

        def test_manifest_pins_outcomes_and_compiler(self):
            """A rerun must be checkable, not just re-runnable: the manifest
            records an outcome hash and the compiler identity it measured."""
            rc, _out, err, doc = self.run_probe()
            self.assertEqual(rc, 0, err)
            self.assertIn("outcomes_sha256", doc)
            self.assertEqual(len(doc["outcomes_sha256"]), 64)
            self.assertEqual(doc["protocol"], PROTOCOL)
            self.assertEqual(doc["strict_env"], "LCCC_STRICT_OPTIONS=1")

            # The hash is over outcomes: flipping one outcome must change it,
            # so a manifest cannot be "reproduced" by a different compiler.
            rc, _out, _err, other = self.run_probe(env={"FAKE_TRIPLE": "aarch64-linux-gnu"})
            self.assertEqual(rc, 0)
            self.assertNotEqual(doc["outcomes_sha256"], other["outcomes_sha256"])

        def test_unprobeable_capability_is_recorded_not_ignored(self):
            """A capability with neither a probe nor a recorded refusal must
            stop the run, not be silently dropped from the claim."""
            rc, _out, err, _doc = self.run_probe(index_invocations=[["mystery-cap"]])
            self.assertEqual(rc, 2, err)
            self.assertIn("no probe for", err)

    suite = unittest.defaultTestLoader.loadTestsFromTestCase(ProbeContract)
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    return 0 if result.wasSuccessful() else 1


def main(argv: list[str] | None = None) -> int:
    if argv is None:
        argv = sys.argv[1:]
    if "--selftest" in argv:
        return _selftest()
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--cc", default="target/fastbuild/lccc")
    ap.add_argument("--index", type=Path, default=DEFAULT_INDEX)
    ap.add_argument("--manifest", type=Path, help="write the evidence JSON here")
    ap.add_argument(
        "--emit-args",
        action="store_true",
        help="print supported capabilities one per line as flag values",
    )
    ap.add_argument(
        "--emit-runner-args",
        action="store_true",
        help="print '--capability <X>' pairs, one argument per line",
    )
    ap.add_argument("--timeout", type=float, default=30.0)
    ap.add_argument(
        "--limit",
        type=int,
        default=0,
        help="probe at most N flags (smoke tests); 0 means all",
    )
    args = ap.parse_args(argv)

    if not args.index.is_file():
        print(f"capability probe: corpus index not found: {args.index}", file=sys.stderr)
        return 2
    cc = shlex.split(args.cc)
    exe = cc[0]
    if not os.path.isabs(exe):
        # Resolve relative to the repository root so the manifest records an
        # absolute path and the probe works from any cwd.
        candidate = (REPO / exe) if (REPO / exe).exists() else Path(exe)
        cc[0] = str(candidate.resolve())
    if not (Path(cc[0]).is_file() and os.access(cc[0], os.X_OK)):
        print(f"capability probe: not an executable: {cc[0]}", file=sys.stderr)
        return 2

    # The probe's whole validity rests on this variable: without it an
    # unimplemented flag is only a warning and every flag would probe "true".
    env = dict(os.environ)
    env["LCCC_STRICT_OPTIONS"] = "1"

    # Self-check the probe's own premise: every control spelling MUST be
    # rejected.  A harness that silently reports supported=True is worse than
    # no harness -- it would bless unimplemented flags and turn every
    # downstream FAIL into a false accusation against the compiler -- and one
    # control spelling only proves the premise for that spelling, which is
    # exactly the hole the `-W` family would slip through (the driver used to
    # discard an unknown warning option's rejection silently).
    #
    # `-l<name>` is NOT a usable control: it is the library-request option, so
    # a driver is right to accept `-lccc-probe-...` (observed while developing
    # this tool).  `-Wno-<unknown>` is not one either: all four reference
    # compilers accept it, so a driver that accepts it is correct -- the strict
    # run is what keeps that class out of the evidence.
    #
    # Every control is evaluated with LCCC_STRICT_OPTIONS=1 exported, so what
    # the controls really pin is that the compiler under test *honours that
    # contract*: one that ignores it cannot certify anything about its own
    # flags, and the probe refuses to emit a manifest rather than a misleading
    # one.
    controls = [
        {**probe_flag(cc, flag, env, args.timeout), "flag": flag, "why_it_must_fail": why}
        for flag, why in NEGATIVE_CONTROLS
    ]
    accepted_controls = [
        {**probe_flag(cc, flag, env, args.timeout), "flag": flag, "why_it_must_pass": why}
        for flag, why in ACCEPTED_CONTROLS
    ]
    controls += accepted_controls
    wrongly_accepted = [
        c["flag"] for c in controls if c.get("why_it_must_fail") and c["supported"]
    ]
    wrongly_refused = [c["flag"] for c in accepted_controls if not c["supported"]]
    if wrongly_accepted or wrongly_refused:
        print(
            "capability probe: FATAL - control(s) "
            f"{wrongly_accepted + wrongly_refused} did not behave as the "
            "contract under test requires (an unimplemented option must be "
            "refused, a satisfied off-request must be accepted), so no "
            "capability list from this compiler would mean anything",
            file=sys.stderr,
        )
        return 2

    index_doc = json.loads(args.index.read_text())
    demanded = _capability_counts(index_doc)
    flags = sorted(c for c in demanded if c.startswith("flag:"))
    if args.limit:
        flags = flags[: args.limit]

    results: dict[str, dict] = {}
    results.update(probe_driver_capabilities(cc, env, args.timeout))
    for cap in flags:
        results[cap] = probe_flag(cc, cap[len("flag:") :], env, args.timeout)

    supported = sorted(c for c, r in results.items() if r["supported"])
    unsupported = {c: r for c, r in results.items() if not r["supported"]}

    # Every capability demanded by the corpus must be either probed here or
    # recorded as deliberately not probeable. Anything else is a silent hole.
    claimed_or_refused = set(results) | set(NOT_PROBEABLE)
    unknown = sorted(set(demanded) - claimed_or_refused)
    if unknown:
        print(
            "capability probe: refusing to proceed - corpus demands capabilities "
            f"this tool has no probe for and no recorded refusal: {unknown[:10]}",
            file=sys.stderr,
        )
        return 2

    coverage = coverage_of(index_doc, set(supported))
    manifest = dict(
        protocol=PROTOCOL,
        compiler=dict(
            command=cc,
            sha256=sha256_file(Path(cc[0])),
            version=run(cc + ["--version"], env, timeout=args.timeout)[1].strip()[:4096],
        ),
        index=dict(path=str(args.index), sha256=sha256_file(args.index)),
        strict_env="LCCC_STRICT_OPTIONS=1",
        controls=controls,
        outcomes_sha256=outcomes_sha256(results),
        supported=supported,
        unsupported=unsupported,
        not_probeable=NOT_PROBEABLE,
        demanded=demanded,
        coverage=coverage,
    )

    if args.manifest:
        args.manifest.parent.mkdir(parents=True, exist_ok=True)
        tmp = args.manifest.with_suffix(args.manifest.suffix + ".tmp")
        tmp.write_text(json.dumps(manifest, indent=1, sort_keys=True) + "\n")
        os.replace(tmp, args.manifest)

    if args.emit_runner_args:
        for cap in supported:
            print("--capability")
            print(cap)
    elif args.emit_args:
        for cap in supported:
            print(cap)

    top_blockers = sorted(
        coverage["blocked_by"].items(), key=lambda kv: (-kv[1], kv[0])
    )[:3]
    print(
        f"capability probe: {len(supported)}/{len(results)} capabilities established; "
        f"{coverage['runnable_invocations']}/{coverage['planned_invocations']} "
        f"planned invocations fully unlocked ({coverage['ratio']:.1%})"
        + (
            "; top blockers: "
            + ", ".join(f"{cap} ({n})" for cap, n in top_blockers)
            if top_blockers
            else ""
        ),
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
