#!/usr/bin/env python3
"""Compare two CTest JUnit reports -- by identity, not by length.

The Expat workload builds the same upstream suite twice, once with GCC and once
with LCCC, and the workload is only a *differential* if both runs executed the
same set of tests.  Counting `<testcase>` elements is not that check: a
candidate configuration that silently dropped `runtests` (or renamed a test, or
gained an extra one) produced the same "2 versus 1, status: pass" summary that a
matching run produces.  Two equal counts over different test sets is the most
convincing kind of nothing.

So this module compares the *multiset of testcase identities* and each test's
status, and it compares the CTest/CMake feature configuration that produced
them -- equal test names must not excuse a weaker candidate build.

Compiler-free by design: `--self-test` runs the whole contract against
in-memory fixtures, which is what makes it a CI gate (`ci_local.sh`
`expat-junit-contract`) rather than a claim about an Expat build.

Usage:
  junit_check.py --reference ctest-gcc.xml --candidate ctest-lccc.xml \
      [--reference-config cmake-cache-gcc.txt --candidate-config cmake-cache-lccc.txt] \
      [--out comparison.json]
  junit_check.py --self-test
"""
from __future__ import annotations

import argparse
import tempfile
import json
import os
import re
import sys
import xml.etree.ElementTree as ET
from collections import Counter
from dataclasses import dataclass
from pathlib import Path

# CTest's `--output-junit` writes <testsuites><testsuite><testcase .../>; older
# or hand-written reports may start at <testsuite>. Both are accepted, and
# nothing else is.
ALLOWED_ROOTS = ("testsuites", "testsuite")
# Every status a `<testcase>` can carry.  A report element we do not model must
# not be readable as a pass: that is how a new CTest failure class becomes a
# silent green.
STATUSES = ("pass", "failure", "error", "skipped")
# Only these CMake cache keys may differ between the two configurations: they
# *are* the point of the differential (which compiler produced the objects) or
# describe where the build happened.  Everything else that changes the compiled
# surface -- EXPAT_* feature switches, build type, generator -- must match.
CONFIG_ALLOWLIST = (
    "CMAKE_C_COMPILER",
    "CMAKE_C_COMPILER_LAUNCHER",
    "CMAKE_C_FLAGS",
    "CMAKE_C_FLAGS_RELEASE",
    "CMAKE_CXX_COMPILER",
    "CMAKE_HOME_DIRECTORY",
    "CMAKE_CACHEFILE_DIR",
    "PROJECT_SOURCE_DIR",
    "PROJECT_BINARY_DIR",
    "CMAKE_CXX_FLAGS",
)


@dataclass(frozen=True)
class Case:
    classname: str
    name: str
    status: str


class ReportError(ValueError):
    """A report that cannot be compared, as opposed to one that differs."""


_ATTR_STATUS = {
    "failed": "failure", "notrun": "skipped", "skipped": "skipped",
    "timeout": "failure", "passed": "pass", "disabled": "skipped",
}


def _status_of(element: ET.Element) -> str:
    """The status of one testcase, from child elements or from attributes.

    CTest's `--output-junit` records a problem as a child element
    (`<failure/>`, `<error/>`, `<skipped/>`); other JUnit writers -- and CTest's
    own `<Test status="notrun">` style -- use an attribute.  Reading only the
    children would call an attribute-flagged failure a pass, which is the exact
    inversion this module must never perform, so both are consulted and an
    unrecognised attribute value is refused instead of defaulted.
    """
    for tag, status in (("failure", "failure"), ("error", "error"), ("skipped", "skipped")):
        if element.find(tag) is not None:
            return status
    for attribute in ("status", "result"):
        value = element.get(attribute)
        if value is None:
            continue
        normalized = _ATTR_STATUS.get(value.strip().lower())
        if normalized is None:
            raise ReportError(f"unrecognised {attribute}={value!r} on a <testcase>; "
                              f"refusing to read an unknown status as a pass")
        return normalized
    return "pass"


def parse_junit(path: Path) -> list[Case]:
    """Every `<testcase>` in a CTest JUnit file, with a usable identity.

    Raises `ReportError` rather than returning a shorter list when the report is
    malformed: an unreadable XML file, no testcases at all, or a testcase
    without a name are all reasons the *comparison* is invalid, not reasons to
    compare fewer tests.
    """
    try:
        root = ET.parse(path).getroot()
    except (OSError, ET.ParseError) as exc:
        raise ReportError(f"{path}: cannot be parsed as JUnit XML: {exc}") from exc
    if root.tag not in ALLOWED_ROOTS:
        raise ReportError(f"{path}: unexpected JUnit root <{root.tag}>, "
                          f"expected one of {list(ALLOWED_ROOTS)}")
    cases: list[Case] = []
    for element in root.iter("testcase"):
        name = (element.get("name") or "").strip()
        if not name:
            raise ReportError(f"{path}: a <testcase> without a name cannot be "
                              f"compared; refusing to treat it as absent or as a pass")
        cases.append(Case((element.get("classname") or "").strip(), name,
                          _status_of(element)))  # ReportError propagates: unknown is fatal
    if not cases:
        raise ReportError(f"{path}: contains no <testcase> elements; an empty CTest "
                          f"report is not a passing differential")
    return cases


def _index(cases: list[Case], label: str) -> dict[tuple[str, str], Counter]:
    index: dict[tuple[str, str], Counter] = {}
    for case in cases:
        index.setdefault((case.classname, case.name), Counter())[case.status] += 1
    for key, statuses in index.items():
        if sum(statuses.values()) > 1 and len(statuses) > 1:
            # Same identity, different recorded status: which one is the run?
            raise ReportError(f"{label}: {key[0]}/{key[1]} appears with mixed statuses "
                              f"{dict(statuses)}; the report is self-contradictory")
    return index


def compare(reference: list[Case], candidate: list[Case], *,
            reference_label: str = "reference", candidate_label: str = "candidate") -> dict:
    """Multiset identity + status comparison, with every difference named."""
    problems: list[str] = []
    ref = _index(reference, reference_label)
    cand = _index(candidate, candidate_label)
    missing = sorted(set(ref) - set(cand))
    extra = sorted(set(cand) - set(ref))
    if missing:
        problems.append(f"{candidate_label} did not run {len(missing)} test(s) the "
                        f"{reference_label} ran: "
                        + ", ".join(f"{a}/{b}" for a, b in missing[:20]))
    if extra:
        problems.append(f"{candidate_label} ran {len(extra)} test(s) the {reference_label} "
                        f"did not: " + ", ".join(f"{a}/{b}" for a, b in extra[:20]))
    status_changes = []
    for key in sorted(set(ref) & set(cand)):
        if ref[key] != cand[key]:
            status_changes.append(f"{key[0]}/{key[1]}: {dict(ref[key])} vs {dict(cand[key])}")
    if status_changes:
        problems.append(f"{len(status_changes)} test(s) changed status: "
                        + "; ".join(status_changes[:20]))
    not_green = []
    for label, cases in ((reference_label, reference), (candidate_label, candidate)):
        bad = [c for c in cases if c.status != "pass"]
        if bad:
            not_green.append(f"{label}: {len(bad)} non-passing test(s), e.g. "
                             + ", ".join(f"{c.classname}/{c.name}({c.status})" for c in bad[:10]))
    if not_green:
        problems.extend(not_green)
    return {
        "comparable": not problems,
        "problems": problems,
        "reference_tests": len(reference),
        "candidate_tests": len(candidate),
        "reference_unique": len(ref),
        "candidate_unique": len(cand),
        "duplicate_identities": {
            reference_label: sum(v for k, v in
                                 Counter((c.classname, c.name) for c in reference).items() if v > 1),
            candidate_label: sum(v for k, v in
                                 Counter((c.classname, c.name) for c in candidate).items() if v > 1),
        },
    }


_CACHE_LINE = re.compile(r"^(?P<key>[^:]+):(?P<type>[^=]*)=(?P<value>.*)$")


def config_manifest(text: str) -> dict[str, str]:
    """`KEY=VALUE` pairs from a CMakeCache.txt that describe the built surface."""
    manifest: dict[str, str] = {}
    for line in text.splitlines():
        match = _CACHE_LINE.match(line.strip())
        if not match:
            continue
        key = match.group("key")
        if key.startswith(("EXPAT_", "CMAKE_BUILD_TYPE", "CMAKE_GENERATOR", "CMAKE_C_FLAGS",
                           "CMAKE_INSTALL_PREFIX", "BUILD_SHARED")):
            manifest[key] = match.group("value")
    return manifest


def compare_config(reference: dict[str, str], candidate: dict[str, str]) -> list[str]:
    """Configuration differences that are NOT the compiler under test."""
    problems = []
    for key in sorted(set(reference) | set(candidate)):
        if any(key == allowed or key.startswith(allowed + "-") for allowed in CONFIG_ALLOWLIST):
            continue
        if reference.get(key) != candidate.get(key):
            problems.append(f"configuration differs on {key}: "
                            f"{reference.get(key)!r} vs {candidate.get(key)!r}")
    return problems


def self_test() -> int:
    """The witnesses this module exists to refuse, plus the control that passes."""
    ok = 0
    ref = [Case("s", "alpha", "pass"), Case("s", "beta", "pass")]
    same = [Case("s", "alpha", "pass"), Case("s", "beta", "pass")]

    def expect(label: str, want_problem: bool, result: dict) -> None:
        nonlocal ok
        got = not result["comparable"]
        if got != want_problem:
            raise AssertionError(f"{label}: expected problem={want_problem}, got {result}")
        ok += 1

    expect("identical suites pass", False, compare(ref, same))
    expect("omitted test is a problem", True, compare(ref, same[:1]))
    expect("equal count, different test is a problem", True,
           compare(ref, [Case("s", "alpha", "pass"), Case("s", "gamma", "pass")]))
    expect("status change is a problem", True,
           compare(ref, [Case("s", "alpha", "failure"), Case("s", "beta", "pass")]))
    expect("skipped is a problem", True,
           compare(ref, [Case("s", "alpha", "skipped"), Case("s", "beta", "pass")]))
    expect("duplicates on one side only are a problem", True,
           compare(ref, [Case("s", "alpha", "pass"), Case("s", "alpha", "pass")]))
    doubled = [Case("s", "alpha", "pass"), Case("s", "alpha", "pass"), Case("s", "beta", "pass")]
    expect("matched duplicates are matched, not masked", False, compare(doubled, list(doubled)))
    mixed = [Case("s", "alpha", "pass"), Case("s", "alpha", "failure"), Case("s", "beta", "pass")]
    try:
        compare(mixed, list(mixed))
    except ReportError:
        ok += 1
    else:
        raise AssertionError("a self-contradictory report must be refused, not compared")
    for label, bad in (("empty report", "<testsuites></testsuites>"),
                       ("unnamed testcase", '<testsuite><testcase classname="s"/></testsuite>'),
                       ("wrong root", "<Foo/>")):
        path = _fixture_text(label + ".xml", bad)
        try:
            parse_junit(path)
        except ReportError:
            ok += 1
        else:
            raise AssertionError(f"{label}: parse_junit accepted it")
    config_problems = compare_config({"EXPAT_BUILD_TESTS": "ON"}, {"EXPAT_BUILD_TESTS": "OFF"})
    if not config_problems:
        raise AssertionError("a feature switch that differs must be a problem")
    ok += 1
    if compare_config({"CMAKE_C_COMPILER": "/usr/bin/gcc"},
                      {"CMAKE_C_COMPILER": "/tmp/lccc"}):
        raise AssertionError("the compiler under test is an allowed difference")
    ok += 1
    print(f"expat JUnit contract: {ok} checks passed")
    return 0


def _fixture_text(name: str, text: str) -> Path:
    """A throwaway report file for the parse-level witnesses."""
    path = Path(tempfile.mkdtemp(prefix="lccc-expat-junit-")) / name
    path.write_text(text)
    return path


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--reference", type=Path)
    ap.add_argument("--candidate", type=Path)
    ap.add_argument("--reference-config", type=Path)
    ap.add_argument("--candidate-config", type=Path)
    ap.add_argument("--out", type=Path)
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args(argv)
    if args.self_test:
        return self_test()
    if not (args.reference and args.candidate):
        ap.error("--reference and --candidate are required (or --self-test)")
    reference = parse_junit(args.reference)
    candidate = parse_junit(args.candidate)
    result = compare(reference, candidate)
    if args.reference_config and args.candidate_config:
        problems = compare_config(config_manifest(args.reference_config.read_text()),
                                  config_manifest(args.candidate_config.read_text()))
        result["configuration_problems"] = problems
        if problems:
            result["comparable"] = False
            result["problems"].extend(problems)
    elif args.reference_config or args.candidate_config:
        ap.error("--reference-config and --candidate-config come in pairs; a "
                 "differential without both configurations is not comparable")
    result["reference_path"] = str(args.reference)
    result["candidate_path"] = str(args.candidate)
    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        # A per-write temp name: the workload's own lock serialises writers, but
        # a fixed `summary.json.tmp` in a shared artifact dir is one stray
        # process away from two writers truncating each other.
        tmp = args.out.with_name(f"{args.out.name}.tmp.{os.getpid()}")
        tmp.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
        tmp.replace(args.out)
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0 if result["comparable"] else 1


if __name__ == "__main__":
    sys.exit(main())
