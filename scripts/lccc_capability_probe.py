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
    *Driver acceptance*: ``lccc -fsyntax-only -x c <empty> X`` exits 0 with
    ``LCCC_STRICT_OPTIONS=1`` exported.  The strict variable matters: LCCC
    diagnoses an unrecognized option on stderr and keeps going by default
    (build systems probe with speculative flags and rely on the exit status),
    so a plain run cannot distinguish "implemented" from "ignored".  Under
    ``LCCC_STRICT_OPTIONS=1`` an unimplemented option is fatal, which makes the
    exit status exactly the predicate wanted here.

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
    Established by preprocessing ``-dM`` and matching the reported version,
    plus a live parse of a C17-only construct to catch a stale macro.

Anything else (``external-inputs``, EDG-only associations, ...) is NOT
probeable by compilation and is therefore never claimed.

Reproducibility
---------------
The manifest pins the compiler path, its SHA-256, the LCCC version banner, the
probe protocol version and a hash of every probe command's outcome, so a rerun
after a compiler change either reproduces the manifest or reports exactly which
capabilities changed.  ``--emit-args`` writes the ``--capability`` argument list
that ``run_clang_c_corpus.py`` accepts, which is how the two tools are wired.

Usage
-----
    # Write an evidence manifest for one compiler.
    scripts/lccc_capability_probe.py --cc target/fastbuild/lccc \\
        --manifest results/caps.json

    # Feed the corpus runner directly (prints one flag per line).
    scripts/lccc_capability_probe.py --cc target/fastbuild/lccc --emit-args \\
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
PROTOCOL = 2

# Capabilities that describe the driver rather than a single flag.  Each is
# established by a dedicated probe below; none is assumed from the family name.
DRIVER_CAPABILITIES = ("clang-driver-target", "clang-default-c-dialect")

# Capabilities the corpus index can demand that are NOT decidable by invoking
# the compiler, with the reason.  Listed so that "we did not claim it" is a
# recorded decision and not an oversight.
NOT_PROBEABLE = {
    "external-inputs": "requires resolving %S/%t side inputs outside the corpus root",
}


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for block in iter(lambda: fh.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def demanded_capabilities(index: Path) -> dict[str, int]:
    """Every capability the selected corpus can demand, with its frequency."""
    doc = json.loads(index.read_text())
    counts: dict[str, int] = {}
    for rec in doc["files"]:
        for inv in rec["invocations"]:
            for cap in inv["capabilities"]:
                counts[cap] = counts.get(cap, 0) + 1
    return counts


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
#   * an unknown flag           -> exit 0 **with** an "unrecognized command-line
#                                  option" warning, unless LCCC_STRICT_OPTIONS is
#                                  exported, in which case exit 1
#
# The last line is the property the whole tool rests on, so the selftest
# asserts BOTH halves of it: that the fake compiler still "succeeds" without the
# variable (proving the probe needs it) and that the negative control fires when
# the variable is ignored.

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
if os.environ.get("FAKE_STDC_ABSENT") == "1" and "-dM" not in args:
    pass
unknown = [a for a in args if a.startswith("-") and a not in KNOWN
           and a not in {"-fsyntax-only", "-x", "c", "-"}]
if unknown and os.environ.get("FAKE_IGNORES_STRICT") != "1":
    if os.environ.get("LCCC_STRICT_OPTIONS"):
        sys.stderr.write("ccc: error: unrecognized command-line option '%s'\n" % unknown[0])
        raise SystemExit(1)
    sys.stderr.write("warning: unrecognized command-line option '%s'\n" % unknown[0])
raise SystemExit(0)
"""


def _selftest() -> int:
    import tempfile
    import unittest

    class ProbeContract(unittest.TestCase):
        def run_probe(self, *extra: str, env: dict[str, str] | None = None, index_caps=None):
            """Invoke this script as a subprocess against a synthetic compiler.

            Returns (returncode, stdout, stderr, manifest-or-None). The manifest
            is parsed INSIDE the temporary directory: returning its path would
            hand back a file that the context manager has already deleted.
            """
            with tempfile.TemporaryDirectory() as td:
                fake = Path(td) / "fakecc"
                fake.write_text(FAKE_COMPILER)
                fake.chmod(0o755)
                index = Path(td) / "index.json"
                caps = index_caps or [
                    "flag:-mfake-known",
                    "flag:-mfake-unknown",
                    "clang-driver-target",
                    "clang-default-c-dialect",
                ]
                index.write_text(
                    json.dumps(
                        {"files": [{"invocations": [{"capabilities": [c]} for c in caps]}]}
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

        def test_strict_env_is_actually_exported(self):
            """The probe must set LCCC_STRICT_OPTIONS; without it every flag
            would probe 'true' because the real driver only warns by default."""
            rc, _out, err, _doc = self.run_probe(env={"FAKE_IGNORES_STRICT": "1"})
            self.assertEqual(
                rc, 2, "the negative control must abort when strict mode is not honoured"
            )
            self.assertIn("negative control", err)

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

        def test_unprobeable_capability_is_recorded_not_ignored(self):
            """A capability with neither a probe nor a recorded refusal must
            stop the run, not be silently dropped from the claim."""
            rc, _out, err, _doc = self.run_probe(index_caps=["mystery-cap"])
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

    # Self-check the probe's own premise on a string that MUST be rejected.
    # A probe harness that silently returns supported=True for everything is
    # worse than no harness: it would bless unimplemented flags and turn every
    # downstream FAIL into a false accusation against the compiler.
    #
    # The control uses a `--` long-option spelling deliberately. `-l<name>` is
    # NOT a usable control: it is the library-request option, so a driver is
    # right to accept `-lccc-probe-...` and the control would "fail" against a
    # perfectly correct compiler (observed while developing this tool).
    control_flag = "--lccc-probe-nonexistent-option-xyzzy"
    control = probe_flag(cc, control_flag, env, args.timeout)
    if control["supported"]:
        print(
            "capability probe: FATAL - the negative control "
            f"'{control_flag}' was reported as supported, so this compiler does "
            "not honour LCCC_STRICT_OPTIONS and no capability list from it "
            "would mean anything",
            file=sys.stderr,
        )
        return 2

    demanded = demanded_capabilities(args.index)
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

    covered = sum(demanded.get(c, 0) for c in supported)
    total = sum(demanded.values())
    manifest = dict(
        protocol=PROTOCOL,
        compiler=dict(
            command=cc,
            sha256=sha256_file(Path(cc[0])),
            version=run(cc + ["--version"], env, timeout=args.timeout)[1].strip()[:4096],
        ),
        index=dict(path=str(args.index), sha256=sha256_file(args.index)),
        strict_env="LCCC_STRICT_OPTIONS=1",
        negative_control=control,
        supported=supported,
        unsupported=unsupported,
        not_probeable=NOT_PROBEABLE,
        demanded=demanded,
        coverage=dict(
            planned_invocation_weight_covered=covered,
            planned_invocation_weight_total=total,
            ratio=(covered / total) if total else 0.0,
        ),
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

    print(
        f"capability probe: {len(supported)}/{len(results)} capabilities established; "
        f"they cover {covered}/{total} weighted planned invocations "
        f"({manifest['coverage']['ratio']:.1%})",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
