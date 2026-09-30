#!/usr/bin/env python3
"""glibc ``make check`` triage harness (BACKLOG MS-11).

Classifies every non-passing glibc test into exactly one bucket so a compiler
engineer can tell *miscompiles* (our bug) from *unsupported features* (our gap)
and *environment* noise (nobody's bug) without reading thousands of ``.out``
files by hand.

    scripts/glibc_check_triage.py BUILD_DIR
    scripts/glibc_check_triage.py BUILD_DIR --baseline GCC_BUILD_DIR
    scripts/glibc_check_triage.py BUILD_DIR --json out.json --markdown out.md
    scripts/glibc_check_triage.py --self-test

``BUILD_DIR`` is a glibc objdir after ``make check``.  Sources of truth, in
order of authority:

1. ``<test>.test-result`` -- ``PASS|FAIL|XFAIL|XPASS|UNSUPPORTED|ERROR: name``
   (glibc's own ``evaluate-test`` output);
2. ``tests.sum`` -- the same lines concatenated (per-test files may be pruned);
3. ``<test>.out`` -- the test's stdout/stderr, scanned for evidence.

Buckets
-------

``miscompile``
    Failure with code-execution evidence (SIGSEGV/SIGILL/SIGBUS/SIGFPE/SIGABRT
    from the test child, ``Assertion ... failed``, ``support/check.h`` compare
    mismatches) and no environment/unsupported evidence.  With ``--baseline``
    it is promoted to ``miscompile-confirmed`` when the identical test PASSes
    in the reference (GCC) build and demoted to ``preexisting`` when it also
    fails there.
``unsupported-feature``
    ``UNSUPPORTED`` results, or diagnostics showing our frontend/assembler/
    linker refused a construct (``unsupported``, ``unknown attribute``,
    ``ifunc``, ``undefined reference to `__builtin``, ``not implemented``).
``environment``
    Timeouts, missing files/locales, permission/namespace failures, ENOSYS,
    ENOMEM, no network, read-only fs.  Outranks everything else (a SIGSEGV
    after "cannot open shared object" is not a miscompile).
``unclassified``
    Failure with no recognised evidence; always listed so nothing is lost.

Exit status: 0 when the ``miscompile*`` count is within ``--allow-miscompiles``
(default 0), 1 otherwise, 2 on usage/IO errors.
"""
from __future__ import annotations

import argparse
import json
import re
import sys
import tempfile
from collections import Counter, defaultdict
from dataclasses import asdict, dataclass, field
from pathlib import Path

RESULT_RE = re.compile(
    r"^(PASS|FAIL|XFAIL|XPASS|UNSUPPORTED|ERROR):\s+(\S+)", re.MULTILINE
)

SIGNAL_EVIDENCE = [
    (re.compile(r"got `?(Segmentation fault|SIGSEGV)"), "SIGSEGV"),
    (re.compile(r"got `?(Illegal instruction|SIGILL)"), "SIGILL"),
    (re.compile(r"got `?(Bus error|SIGBUS)"), "SIGBUS"),
    (re.compile(r"got `?(Floating point exception|SIGFPE)"), "SIGFPE"),
    (re.compile(r"got `?(Aborted|SIGABRT)"), "SIGABRT"),
    (re.compile(r"Didn't expect signal from child"), "signal"),
    (re.compile(r"Assertion [`'\"].*failed|assertion failed", re.IGNORECASE), "assert"),
    (re.compile(r"^error: .*\(.*\) (!=|==|<|>|<=|>=) ", re.MULTILINE), "check-mismatch"),
    (re.compile(r"^error: .*:\d+: .*(mismatch|unexpected|wrong)",
                re.MULTILINE | re.IGNORECASE), "check-mismatch"),
]
UNSUPPORTED_EVIDENCE = [
    re.compile(r"\bunsupported\b", re.IGNORECASE),
    re.compile(r"unknown attribute|attribute .* not supported", re.IGNORECASE),
    re.compile(r"\bifunc\b|gnu_indirect_function", re.IGNORECASE),
    re.compile(r"not implemented|not yet implemented|unimplemented", re.IGNORECASE),
    re.compile(r"undefined reference to [`']__(builtin|sync|atomic)"),
    re.compile(r"unknown (builtin|directive|relocation)", re.IGNORECASE),
    re.compile(r"__float128|_Float128|_Decimal(32|64|128)"),
]
ENVIRONMENT_EVIDENCE = [
    re.compile(r"Timed out|timeout|killed the child process", re.IGNORECASE),
    re.compile(r"cannot open shared object file|No such file or directory"),
    re.compile(r"Permission denied|Operation not permitted|EPERM"),
    re.compile(r"Function not implemented|ENOSYS"),
    re.compile(r"locale.*not (found|available)|cannot (set|load) locale|setlocale"),
    re.compile(r"unshare|user namespace|CLONE_NEW|/proc/.*(No such|cannot)"),
    re.compile(r"Cannot allocate memory|Out of memory|ENOMEM"),
    re.compile(r"Network is unreachable|Name or service not known|Temporary failure"),
    re.compile(r"Read-only file system|No space left on device"),
]

FAILING = {"FAIL", "ERROR", "XPASS"}
NONFAIL = {"PASS", "XFAIL"}


@dataclass
class Verdict:
    test: str
    result: str
    bucket: str
    evidence: list[str] = field(default_factory=list)
    baseline_result: str | None = None


def parse_results(text: str) -> dict[str, str]:
    """``{test: result}`` from ``tests.sum``/``.test-result`` text (last wins)."""
    return {m.group(2): m.group(1) for m in RESULT_RE.finditer(text)}


def collect_results(build: Path) -> dict[str, str]:
    results: dict[str, str] = {}
    summary = build / "tests.sum"
    if summary.is_file():
        results.update(parse_results(summary.read_text(errors="replace")))
    for tr in sorted(build.rglob("*.test-result")):
        results.update(parse_results(tr.read_text(errors="replace")))
    return results


def read_out(build: Path, test: str) -> str:
    path = build / f"{test}.out"
    return path.read_text(errors="replace") if path.is_file() else ""


def classify(result: str, log: str) -> tuple[str, list[str]]:
    """Map one (result, log) pair to ``(bucket, evidence)``."""
    if result in NONFAIL:
        return result.lower(), []
    if result == "UNSUPPORTED":
        return "unsupported-feature", ["glibc reported UNSUPPORTED"]
    env = [p.pattern for p in ENVIRONMENT_EVIDENCE if p.search(log)]
    uns = [p.pattern for p in UNSUPPORTED_EVIDENCE if p.search(log)]
    sig = [name for rx, name in SIGNAL_EVIDENCE if rx.search(log)]
    if env:
        return "environment", env[:3]
    if uns:
        return "unsupported-feature", uns[:3]
    if sig:
        return "miscompile", sorted(set(sig))
    return "unclassified", ["no recognised evidence"]


def triage(build: Path, baseline: Path | None) -> list[Verdict]:
    results = collect_results(build)
    base = collect_results(baseline) if baseline else {}
    verdicts: list[Verdict] = []
    for test in sorted(results):
        result = results[test]
        bucket, evidence = classify(result, read_out(build, test))
        base_result = base.get(test) if baseline else None
        if baseline and bucket == "miscompile":
            if base_result in NONFAIL:
                bucket = "miscompile-confirmed"
                evidence.append(f"reference build: {base_result}")
            elif base_result in FAILING or base_result == "UNSUPPORTED":
                bucket = "preexisting"
                evidence.append(f"reference build also: {base_result}")
        verdicts.append(Verdict(test, result, bucket, evidence, base_result))
    return verdicts


def summarize(verdicts: list[Verdict]) -> dict[str, object]:
    counts = Counter(v.bucket for v in verdicts)
    by_dir: dict[str, Counter[str]] = defaultdict(Counter)
    for v in verdicts:
        if v.bucket not in ("pass", "xfail"):
            by_dir[v.test.split("/")[0]][v.bucket] += 1
    return {
        "total": len(verdicts),
        "counts": dict(sorted(counts.items())),
        "by_directory": {d: dict(c) for d, c in sorted(by_dir.items())},
        "miscompiles": sorted(v.test for v in verdicts if v.bucket.startswith("miscompile")),
    }


def render_markdown(verdicts: list[Verdict], summary: dict[str, object]) -> str:
    counts = summary["counts"]
    assert isinstance(counts, dict)
    lines = ["# glibc `make check` triage", "", "| bucket | tests |", "|---|---:|"]
    lines += [f"| {bucket} | {n} |" for bucket, n in counts.items()]
    lines += ["", "## Non-passing tests", "", "| test | result | bucket | evidence |",
              "|---|---|---|---|"]
    for v in verdicts:
        if v.bucket in ("pass", "xfail"):
            continue
        ev = "; ".join(e.replace("|", "\\|") for e in v.evidence)
        lines.append(f"| `{v.test}` | {v.result} | {v.bucket} | {ev} |")
    return "\n".join(lines) + "\n"


def _write(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


def self_test() -> int:
    """Fixture-driven check of every bucket and the baseline promotion rules."""
    with tempfile.TemporaryDirectory() as d:
        root = Path(d)
        build, ref = root / "lccc", root / "gcc"
        _write(build / "tests.sum", "\n".join([
            "PASS: string/test-memcmp", "FAIL: string/tst-crash", "FAIL: elf/tst-ifunc",
            "FAIL: misc/tst-env", "UNSUPPORTED: nptl/tst-skip", "FAIL: math/test-weird",
            "FAIL: stdio-common/tst-both", "XFAIL: posix/tst-known"]) + "\n")
        _write(ref / "tests.sum", "PASS: string/tst-crash\nFAIL: stdio-common/tst-both\n")
        _write(build / "string/tst-crash.out",
               "Didn't expect signal from child: got `Segmentation fault'\n")
        _write(build / "elf/tst-ifunc.out",
               "error: ifunc relocation\nDidn't expect signal from child: got `Segmentation fault'\n")
        _write(build / "misc/tst-env.out", "Timed out: killed the child process\n")
        _write(build / "stdio-common/tst-both.out", "Assertion `x == 3' failed.\n")
        verdicts = {v.test: v for v in triage(build, ref)}
        expect = {
            "string/test-memcmp": "pass",
            "string/tst-crash": "miscompile-confirmed",
            "elf/tst-ifunc": "unsupported-feature",
            "misc/tst-env": "environment",
            "nptl/tst-skip": "unsupported-feature",
            "math/test-weird": "unclassified",
            "stdio-common/tst-both": "preexisting",
            "posix/tst-known": "xfail",
        }
        for test, bucket in expect.items():
            if verdicts[test].bucket != bucket:
                print(f"self-test FAIL {test}: {verdicts[test].bucket} != {bucket}",
                      file=sys.stderr)
                return 1
        if summarize(list(verdicts.values()))["miscompiles"] != ["string/tst-crash"]:
            print("self-test FAIL: miscompile list", file=sys.stderr)
            return 1
    print("glibc_check_triage self-test OK")
    return 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=(__doc__ or "").split("\n\n")[0])
    ap.add_argument("build", nargs="?", type=Path, help="glibc objdir after make check")
    ap.add_argument("--baseline", type=Path, help="reference (GCC) objdir")
    ap.add_argument("--json", type=Path, dest="json_out")
    ap.add_argument("--markdown", type=Path)
    ap.add_argument("--allow-miscompiles", type=int, default=0,
                    help="tolerated count of miscompile* tests (default 0)")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args(argv)
    if args.self_test:
        return self_test()
    if args.build is None or not args.build.is_dir():
        print("error: BUILD_DIR required and must exist", file=sys.stderr)
        return 2
    verdicts = triage(args.build, args.baseline)
    if not verdicts:
        print("error: no test results found (run `make check` first)", file=sys.stderr)
        return 2
    summary = summarize(verdicts)
    if args.json_out:
        args.json_out.write_text(json.dumps(
            {"summary": summary, "tests": [asdict(v) for v in verdicts]}, indent=2) + "\n")
    md = render_markdown(verdicts, summary)
    if args.markdown:
        args.markdown.write_text(md)
    else:
        sys.stdout.write(md)
    bad = len(summary["miscompiles"])  # type: ignore[arg-type]
    return 0 if bad <= args.allow_miscompiles else 1


if __name__ == "__main__":
    sys.exit(main())
